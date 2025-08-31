use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use anyhow::Context;
use btleplug::api::{
    AdvertisementData, AdvertisingOptions, Central, CentralEvent, Manager as _, Peripheral as _,
    ScanFilter,
};
use btleplug::platform::Manager;
use clap::{Parser, Subcommand};
use rand::Rng;
use tokio::{select, time::sleep};

mod crypto;

const COMPANY_ID: u16 = 0xFFFF; // experimental/manufacturer data key
const VER: u8 = 1;
const MAX_PAYLOAD: usize = 20; // leave header room; BLE legacy adv ~31B total

#[derive(Parser, Debug)]
#[command(
    name = "ble-chirp",
    about = "Broadcast/scan tiny messages via BLE advertising (mesh-style)"
)]
struct Args {
    /// Which adapter index to use (0 by default)
    #[arg(long, default_value_t = 0)]
    adapter: usize,
    /// Passphrase for payload encryption/decryption
    #[arg(long)]
    passphrase: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Transmit a message (advertise chunked frames)
    Tx {
        /// Topic/channel (0-255)
        #[arg(long, default_value_t = 7)]
        topic: u8,
        /// Hops to relay
        #[arg(long, default_value_t = 3)]
        ttl: u8,
        /// Message text
        msg: String,
        /// Milliseconds to advertise each chunk
        #[arg(long, default_value_t = 500)]
        dwell_ms: u64,
    },
    /// Receive & show messages; optionally relay them
    Rx {
        /// Only show a specific topic
        #[arg(long)]
        topic: Option<u8>,
        /// Relay chunks with TTL-1 (gossip)
        #[arg(long, default_value_t = true)]
        relay: bool,
    },
}

#[derive(Clone)]
struct Frame {
    topic: u8,
    ttl: u8,
    msg_id: [u8; 4],
    seq: u8,
    tot: u8,
    payload: Vec<u8>,
}

fn pack_frame(f: &Frame) -> Vec<u8> {
    // CompanyID(2 LE) + ver(1)+topic(1)+ttl(1)+msgId(4)+seq(1)+tot(1)+payload
    let mut b = Vec::with_capacity(2 + 1 + 1 + 1 + 4 + 1 + 1 + f.payload.len());
    b.extend_from_slice(&COMPANY_ID.to_le_bytes());
    b.push(VER);
    b.push(f.topic);
    b.push(f.ttl);
    b.extend_from_slice(&f.msg_id);
    b.push(f.seq);
    b.push(f.tot);
    b.extend_from_slice(&f.payload);
    b
}

fn unpack_frame(md: &[u8]) -> Option<Frame> {
    if md.len() < 2 + 1 + 1 + 1 + 4 + 1 + 1 {
        return None;
    }
    let cid = u16::from_le_bytes([md[0], md[1]]);
    if cid != COMPANY_ID {
        return None;
    }
    let ver = md[2];
    if ver != VER {
        return None;
    }
    let topic = md[3];
    let ttl = md[4];
    let msg_id = [md[5], md[6], md[7], md[8]];
    let seq = md[9];
    let tot = md[10];
    let payload = md[11..].to_vec();
    Some(Frame {
        topic,
        ttl,
        msg_id,
        seq,
        tot,
        payload,
    })
}

fn chunk_message(bytes: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
    // returns (seq, tot, payload)
    let tot = ((bytes.len() + MAX_PAYLOAD - 1) / MAX_PAYLOAD).max(1) as u8;
    let mut v = Vec::new();
    for i in 0..tot {
        let s = (i as usize) * MAX_PAYLOAD;
        let e = (s + MAX_PAYLOAD).min(bytes.len());
        v.push((i, tot, bytes[s..e].to_vec()));
    }
    v
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let manager = Manager::new().await.context("btleplug Manager::new")?;
    let adapters = manager.adapters().await.context("list adapters")?;
    let adapter = adapters
        .get(args.adapter)
        .ok_or_else(|| anyhow::anyhow!("adapter {} not found", args.adapter))?
        .clone();

    let key = args.passphrase.as_ref().map(|p| crypto::derive_key(p));

    match args.cmd {
        Cmd::Tx {
            topic,
            ttl,
            msg,
            dwell_ms,
        } => tx(adapter, topic, ttl, &msg, dwell_ms, key).await?,
        Cmd::Rx { topic, relay } => rx(adapter, topic, relay, key).await?,
    }
    Ok(())
}

async fn tx(
    adapter: btleplug::platform::Adapter,
    topic: u8,
    ttl: u8,
    msg: &str,
    dwell_ms: u64,
    key: Option<[u8; 32]>,
) -> anyhow::Result<()> {
    let msg_bytes = msg.as_bytes();
    let chunks = chunk_message(msg_bytes);
    let peripheral = adapter.peripheral().await.context("create peripheral")?;
    let msg_id: [u8; 4] = rand::thread_rng().r#gen();
    println!(
        "TX topic={} ttl={} chunks={} msg_id={:02x?}",
        topic,
        ttl,
        chunks.len(),
        msg_id
    );

    for (seq, tot, payload) in chunks {
        let payload = if let Some(ref k) = key {
            crypto::encrypt(k, &msg_id, seq, &payload)?
        } else {
            payload
        };
        let f = Frame {
            topic,
            ttl,
            msg_id,
            seq,
            tot,
            payload,
        };
        let md = pack_frame(&f);
        let mut m = HashMap::new();
        m.insert(COMPANY_ID, md);

        let adv = AdvertisementData {
            local_name: Some("chirp".into()),
            manufacturer_data: Some(m),
            service_data: None,
            services: None,
            appearance: None,
            tx_power_level: None,
            solicited_services: None,
        };

        peripheral
            .start_advertising(adv, AdvertisingOptions::default())
            .await?;
        sleep(Duration::from_millis(dwell_ms)).await;
        peripheral.stop_advertising().await?;
        sleep(Duration::from_millis(60)).await; // small gap between chunks
    }
    println!("Done.");
    Ok(())
}

async fn rx(
    adapter: btleplug::platform::Adapter,
    topic_filter: Option<u8>,
    relay: bool,
    key: Option<[u8; 32]>,
) -> anyhow::Result<()> {
    // de-dupe cache: seen (msg_id, seq)
    let mut seen: VecDeque<([u8; 4], u8)> = VecDeque::with_capacity(2048);
    let mut reasm: HashMap<[u8; 4], (u8, HashMap<u8, Vec<u8>>, u8)> = HashMap::new();
    //                               tot,    parts(seq->payload), topic

    adapter.start_scan(ScanFilter::default()).await?;
    println!(
        "Listening... {}",
        topic_filter
            .map(|t| format!("(topic={})", t))
            .unwrap_or_default()
    );

    let mut events = adapter.events().await?;
    while let Some(evt) = events.recv().await {
        if let CentralEvent::ManufacturerDataAdvertisement {
            manufacturer_data, ..
        } = evt
        {
            if let Some(md) = manufacturer_data.get(&COMPANY_ID) {
                if let Some(mut f) = unpack_frame(md) {
                    if let Some(t) = topic_filter {
                        if f.topic != t {
                            continue;
                        }
                    }

                    // de-dupe (msg_id, seq)
                    if seen.iter().any(|(id, s)| *id == f.msg_id && *s == f.seq) {
                        continue;
                    }
                    if seen.len() >= 2048 {
                        seen.pop_front();
                    }
                    seen.push_back((f.msg_id, f.seq));

                    // decrypt if needed
                    let mut payload = f.payload.clone();
                    if let Some(ref k) = key {
                        match crypto::decrypt(k, &f.msg_id, f.seq, &f.payload) {
                            Ok(p) => payload = p,
                            Err(_) => continue,
                        }
                    }

                    // reassembly
                    let entry = reasm
                        .entry(f.msg_id)
                        .or_insert_with(|| (f.tot, HashMap::new(), f.topic));
                    entry.1.insert(f.seq, payload);

                    // complete?
                    if entry.1.len() as u8 == entry.0 {
                        let mut bytes = Vec::new();
                        for i in 0..entry.0 {
                            if let Some(p) = entry.1.get(&i) {
                                bytes.extend_from_slice(p);
                            }
                        }
                        let text = String::from_utf8_lossy(&bytes);
                        let id8 = hex::encode(f.msg_id);
                        println!("[topic {}] #{}: {}", entry.2, &id8[..8], text);
                        reasm.remove(&f.msg_id);
                    }

                    // relay
                    if relay && f.ttl > 0 {
                        f.ttl -= 1;
                        let backoff = 100 + rand::thread_rng().gen_range(0..400); // 100–500ms
                        tokio::spawn(do_relay(adapter.clone(), f, backoff));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn do_relay(adapter: btleplug::platform::Adapter, f: Frame, backoff_ms: u64) {
    sleep(Duration::from_millis(backoff_ms)).await;
    let peripheral = match adapter.peripheral().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("relay peripheral err: {e}");
            return;
        }
    };
    let mut m = HashMap::new();
    m.insert(COMPANY_ID, pack_frame(&f));
    let adv = AdvertisementData {
        local_name: Some("chirp".into()),
        manufacturer_data: Some(m),
        service_data: None,
        services: None,
        appearance: None,
        tx_power_level: None,
        solicited_services: None,
    };
    if let Err(e) = peripheral
        .start_advertising(adv, AdvertisingOptions::default())
        .await
    {
        eprintln!("relay start adv err: {e}");
        return;
    }
    sleep(Duration::from_millis(300)).await;
    let _ = peripheral.stop_advertising().await;
}
