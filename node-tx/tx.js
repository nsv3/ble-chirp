// node-tx/tx.js
const bleno  = require('@abandonware/bleno');
const crypto = require('node:crypto');

const COMPANY_ID = 0xFFFF;
const VER = 1;
const MAX_PAYLOAD = 20; // final per-frame payload size limit

function parseArgs() {
  const args = process.argv.slice(2);
  let topic = 7, ttl = 3, passphrase = null, textParts = [];
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === '--topic') topic = parseInt(args[++i], 10);
    else if (a.startsWith('--topic=')) topic = parseInt(a.split('=')[1], 10);
    else if (a === '--ttl') ttl = parseInt(args[++i], 10);
    else if (a.startsWith('--ttl=')) ttl = parseInt(a.split('=')[1], 10);
    else if (a === '--pass' || a === '--passphrase') passphrase = args[++i];
    else if (a.startsWith('--pass=')) passphrase = a.split('=')[1];
    else textParts.push(a);
  }
  if (textParts.length === 0) textParts = ['hello from node'];
  return { topic, ttl, passphrase, text: textParts.join(' ') };
}

function chunk(buf, size) {
  const out = [];
  for (let i = 0; i < buf.length; i += size) out.push(buf.subarray(i, i + size));
  return out;
}

function packFrame({ topic, ttl, msgId, seq, tot, payload }) {
  const b = Buffer.alloc(2 + 1 + 1 + 1 + 4 + 1 + 1 + payload.length);
  b.writeUInt16LE(COMPANY_ID, 0);
  let off = 2;
  b.writeUInt8(VER, off++);      // ver
  b.writeUInt8(topic, off++);    // topic
  b.writeUInt8(ttl, off++);      // ttl
  msgId.copy(b, off); off += 4;  // msgId
  b.writeUInt8(seq, off++);      // seq
  b.writeUInt8(tot, off++);      // tot
  payload.copy(b, off);
  return b;
}

function deriveKey(passphrase) {
  if (!passphrase) return null;
  return crypto.createHash('sha256').update(passphrase, 'utf8').digest(); // 32 bytes
}

function encrypt(key, msgId, seq, payload) {
  // Nonce: 12 bytes, msgId in first 4, seq at index 4, rest zeroed
  const nonce = Buffer.alloc(12, 0);
  msgId.copy(nonce, 0, 0, 4);
  nonce[4] = seq & 0xff;
  const cipher = crypto.createCipheriv('chacha20-poly1305', key, nonce, { authTagLength: 16 });
  const ct = Buffer.concat([cipher.update(payload), cipher.final()]);
  const tag = cipher.getAuthTag();
  return Buffer.concat([ct, tag]); // matches Rust (ct || tag)
}

async function main() {
  const { topic, ttl, passphrase, text } = parseArgs();
  const key   = deriveKey(passphrase);
  const msg   = Buffer.from(text, 'utf8');
  const msgId = crypto.randomBytes(4);
  const ENC_OVERHEAD = key ? 16 : 0;
  const CHUNK_SIZE = Math.max(1, MAX_PAYLOAD - ENC_OVERHEAD);
  const parts = chunk(msg, CHUNK_SIZE);

  await new Promise(res => bleno.on('stateChange', s => (s === 'poweredOn' && res())));
  console.log(`Advertising topic=${topic} ttl=${ttl} chunks=${parts.length}${key ? ' (encrypted)' : ''}`);

  for (let i = 0; i < parts.length; i++) {
    const payload = key ? encrypt(key, msgId, i, parts[i]) : parts[i];
    const frame = packFrame({ topic, ttl, msgId, seq: i, tot: parts.length, payload });
    bleno.startAdvertising('chirp', [], { manufacturerData: frame }, err => {
      if (err) console.error('adv err', err);
    });
    await new Promise(r => setTimeout(r, 10000)); 
    bleno.stopAdvertising();
    await new Promise(r => setTimeout(r, 80));
  }
  console.log('done');
  process.exit(0);
}

main().catch(console.error);
