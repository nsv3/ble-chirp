

use sha2::{Digest, Sha256};


use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use anyhow::Context;
use btleplug::api::{Central, CentralEvent, Manager as _, ScanFilter};
use btleplug::platform::Manager;
use clap::{Parser, Subcommand};
use rand::Rng;
use tokio::time::sleep;
use futures::StreamExt;

mod chat_ui;

mod crypto;

mod rate_limiter;
use rate_limiter::RateLimiter;

const COMPANY_ID: u16 = 0xFFFF; // manufacturer data key
const VER: u8 = 1;
const MAX_PAYLOAD: usize = 20; 

#[derive(Parser, Debug)]
#[command(
    name = "ble-chirp",
    about = "Broadcast/scan tiny messages via BLE advertising (mesh-style)"
)]
struct Args {
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

        #[arg(long, default_value_t = 7, conflicts_with = "room")]
        topic: u8,
        #[arg(long)]
        room: Option<String>,
        #[arg(long, default_value_t = 3)]
        ttl: u8,
        msg: String,
        #[arg(long, default_value_t = 500)]
        dwell_ms: u64,
        #[arg(long, default_value_t = 2.0)]
        rate: f64,
    },
    Rx {

        #[arg(long, conflicts_with = "room")]
        topic: Option<u8>,
        #[arg(long)]
        room: Option<String>,
        #[arg(long, default_value_t = true)]
        relay: bool,
    },


    Chat {
        #[arg(long, default_value_t = 7, conflicts_with = "room")]
        topic: u8,
        #[arg(long)]
        room: Option<String>,
        #[arg(long, default_value_t = 3)]
        ttl: u8,
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

    let mut i = 0usize;

    if md.len() >= 2 {
        let cid = u16::from_le_bytes([md[0], md[1]]);
        if cid == COMPANY_ID {
            i = 2;
        }
    }

    if md.len() < i + 1 + 1 + 1 + 4 + 1 + 1 {
        return None;
    }

    let ver = md[i];
    if ver != VER {
        return None;
    }
    i += 1;

    let topic = md[i];
    i += 1;
    let ttl = md[i];
    i += 1;

    let msg_id = [md[i], md[i + 1], md[i + 2], md[i + 3]];
    i += 4;

    let seq = md[i];
    i += 1;
    let tot = md[i];
    i += 1;

    let payload = md[i..].to_vec();
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
    let tot = ((bytes.len() + MAX_PAYLOAD - 1) / MAX_PAYLOAD).max(1) as u8;
    let mut v = Vec::new();
    for i in 0..tot {
        let s = (i as usize) * MAX_PAYLOAD;
        let e = (s + MAX_PAYLOAD).min(bytes.len());
        v.push((i, tot, bytes[s..e].to_vec()));
    }
    v
}

fn topic_from_room(room: &str) -> u8 {
    let mut h = Sha256::new();
    h.update(room.as_bytes());
    let digest = h.finalize();
    digest[0]
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
            room,
            ttl,
            msg,
            dwell_ms,
            rate,
        } => {
            let topic = room.map_or(topic, |r| topic_from_room(&r));
            tx(adapter, topic, ttl, &msg, dwell_ms, rate, key).await?
        }
        Cmd::Rx { topic, room, relay } => {
            let topic = match (topic, room) {
                (Some(t), _) => Some(t),
                (_, Some(r)) => Some(topic_from_room(&r)),
                _ => None,
            };
            rx(adapter, topic, relay, key).await?
        }
        Cmd::Chat { topic, room, ttl } => {
            let topic = room.map_or(topic, |r| topic_from_room(&r));
            chat_ui::chat(adapter, topic, ttl, key, 2.0).await?
        }
    }
    Ok(())
}

pub(crate) async fn tx(
    adapter: btleplug::platform::Adapter,
    topic: u8,
    ttl: u8,
    msg: &str,
    dwell_ms: u64,
    rate: f64,
    key: Option<crypto::KeyBytes>,

) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        anyhow::bail!(
            "Advertising is not supported by btleplug on macOS. Use the Node sender below to test TX."
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        let msg_bytes = msg.as_bytes();
        let chunks = chunk_message(msg_bytes);
        let peripheral = adapter.peripheral().await.context("create peripheral")?;
        let msg_id = rand::random::<[u8; 4]>();
        println!(
            "TX topic={} ttl={} chunks={} msg_id={:02x?}",
            topic,
            ttl,
            chunks.len(),
            msg_id
        );

        let mut rl = RateLimiter::new(rate);
        for (seq, tot, mut payload) in chunks {
            rl.acquire().await;
            if let Some(ref k) = key {
                payload = crypto::encrypt(k, &msg_id, seq, &payload)
                    .context("encrypt payload")?;
            }
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

            use btleplug::api::{AdvertisementData, AdvertisingOptions};
            peripheral
                .start_advertising(
                    AdvertisementData {
                        local_name: Some("chirp".into()),
                        manufacturer_data: Some(m),
                        service_data: None,
                        services: None,
                        appearance: None,
                        tx_power_level: None,
                        solicited_services: None,
                    },
                    AdvertisingOptions::default(),
                )
                .await?;
            sleep(Duration::from_millis(dwell_ms)).await;
            peripheral.stop_advertising().await?;
            sleep(Duration::from_millis(60)).await; 
        }
        println!("Done.");
        Ok(())
    }
}

pub(crate) async fn rx_loop<F>(
    adapter: btleplug::platform::Adapter,
    topic_filter: Option<u8>,
    relay: bool,
    key: Option<crypto::KeyBytes>,


    mut on_msg: F,
) -> anyhow::Result<()>
where
    F: FnMut(u8, [u8; 4], String) + Send + 'static,
{
    
    let mut seen: VecDeque<([u8; 4], u8)> = VecDeque::with_capacity(2048);
    let mut reasm: HashMap<[u8; 4], (u8, HashMap<u8, Vec<u8>>, u8)> = HashMap::new();
    

    adapter.start_scan(ScanFilter::default()).await?;
    println!(
        "Listening... {}",
        topic_filter
            .map(|t| format!("(topic={})", t))
            .unwrap_or_default()
    );

    let mut events = adapter.events().await?;
    while let Some(evt) = events.next().await {
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

                    
                    if seen.iter().any(|(id, s)| *id == f.msg_id && *s == f.seq) {
                        continue;
                    }
                    if seen.len() >= 2048 {
                        seen.pop_front();
                    }
                    seen.push_back((f.msg_id, f.seq));

                    
                    let mut payload = f.payload.clone();
                    if let Some(ref k) = key {
                        match crypto::decrypt(k, &f.msg_id, f.seq, &f.payload) {
                            Ok(p) => payload = p,
                            Err(_) => continue,
                        }
                    }

                    
                    let entry = reasm
                        .entry(f.msg_id)
                        .or_insert_with(|| (f.tot, HashMap::new(), f.topic));
                    entry.1.insert(f.seq, payload);

                    
                    if entry.1.len() as u8 == entry.0 {
                        let mut bytes = Vec::new();
                        for i in 0..entry.0 {
                            if let Some(p) = entry.1.get(&i) {
                                bytes.extend_from_slice(p);
                            }
                        }

                        let text = String::from_utf8_lossy(&bytes).to_string();
                        on_msg(entry.2, f.msg_id, text);
                        reasm.remove(&f.msg_id);
                    }

                    
                    if relay && f.ttl > 0 {
                        f.ttl -= 1;
                        let backoff = 100 + rand::thread_rng().gen_range(0..400); 
                        tokio::spawn(do_relay(adapter.clone(), f, backoff));
                    }
                }
            }
        }
    }
    Ok(())
}

async fn rx(
    adapter: btleplug::platform::Adapter,
    topic_filter: Option<u8>,
    relay: bool,
    key: Option<crypto::KeyBytes>,
) -> anyhow::Result<()> {
    rx_loop(adapter, topic_filter, relay, key, |topic, id, text| {
        let id8 = hex::encode(id);
        println!("[topic {}] #{}: {}", topic, &id8[..8], text);
    })
    .await
}

async fn do_relay(adapter: btleplug::platform::Adapter, f: Frame, backoff_ms: u64) {
    #[cfg(target_os = "macos")]
    {
        eprintln!("relay disabled: advertising not supported on macOS via btleplug");
        return;
    }

    #[cfg(not(target_os = "macos"))]
    {
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
        use btleplug::api::{AdvertisementData, AdvertisingOptions};
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
}
