use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;

#[derive(Debug, Clone, Copy)]
enum Direction {
    Rx,
    Tx,
}

#[derive(Debug)]
struct CaptureEvent {
    dir: Direction,
    ts: SystemTime,
    frame: Vec<u8>,
}

/// Background pcap writer with size+time rolling.
///
/// Format: classic PCAP, linktype Ethernet (DLT_EN10MB).
pub struct PcapCapture {
    tx: Sender<CaptureEvent>,
}

impl PcapCapture {
    pub fn start(
        out_dir: PathBuf,
        rotate_max_bytes: u64,
        rotate_interval: Duration,
    ) -> anyhow::Result<Arc<Self>> {
        fs::create_dir_all(&out_dir)
            .with_context(|| format!("create capture dir {}", out_dir.display()))?;

        let (tx, rx) = mpsc::channel::<CaptureEvent>();
        thread::Builder::new()
            .name("ntx-pcap-writer".to_string())
            .spawn(move || {
                if let Err(e) = writer_thread(out_dir, rotate_max_bytes, rotate_interval, rx) {
                    tracing::error!(target: "ntx::capture", error = %e, error_dbg = ?e, "pcap writer thread exited");
                }
            })
            .context("spawn pcap writer thread")?;

        Ok(Arc::new(Self { tx }))
    }

    pub fn record_rx(&self, frame: &[u8]) {
        self.send(Direction::Rx, frame);
    }

    pub fn record_tx(&self, frame: &[u8]) {
        self.send(Direction::Tx, frame);
    }

    fn send(&self, dir: Direction, frame: &[u8]) {
        // Best-effort: drop if the writer thread is gone.
        let _ = self.tx.send(CaptureEvent {
            dir,
            ts: SystemTime::now(),
            frame: frame.to_vec(),
        });
    }
}

fn writer_thread(
    out_dir: PathBuf,
    rotate_max_bytes: u64,
    rotate_interval: Duration,
    rx: mpsc::Receiver<CaptureEvent>,
) -> anyhow::Result<()> {
    let snaplen: u32 = 65535;
    let linktype_eth: u32 = 1; // DLT_EN10MB

    let mut seq: u64 = 0;
    let mut cur = open_new_pcap(&out_dir, snaplen, linktype_eth, seq)?;
    let mut opened_at = SystemTime::now();
    let mut bytes_written: u64 = cur.bytes_written;

    while let Ok(ev) = rx.recv() {
        // Time-based rotate.
        if rotate_interval.as_secs() > 0 {
            if opened_at.elapsed().unwrap_or_default() >= rotate_interval {
                seq += 1;
                cur = open_new_pcap(&out_dir, snaplen, linktype_eth, seq)?;
                opened_at = SystemTime::now();
                bytes_written = cur.bytes_written;
            }
        }

        // Size-based rotate.
        if rotate_max_bytes > 0 && bytes_written >= rotate_max_bytes {
            seq += 1;
            cur = open_new_pcap(&out_dir, snaplen, linktype_eth, seq)?;
            opened_at = SystemTime::now();
            bytes_written = cur.bytes_written;
        }

        // Note: classic PCAP doesn't encode direction; we currently write both RX/TX
        // into the same capture file.
        let _dir = ev.dir;

        let (ts_sec, ts_usec) = to_pcap_timestamp(ev.ts);
        let orig_len = ev.frame.len() as u32;
        let incl_len = orig_len.min(snaplen);

        // Record header.
        cur.w.write_all(&ts_sec.to_le_bytes())?;
        cur.w.write_all(&ts_usec.to_le_bytes())?;
        cur.w.write_all(&incl_len.to_le_bytes())?;
        cur.w.write_all(&orig_len.to_le_bytes())?;
        cur.w.write_all(&ev.frame[..incl_len as usize])?;

        bytes_written = bytes_written
            .saturating_add(16)
            .saturating_add(incl_len as u64);

        // Flush occasionally (cheap enough for now; can be tuned later).
        cur.w.flush()?;
    }

    Ok(())
}

struct PcapFile {
    w: BufWriter<File>,
    bytes_written: u64,
}

fn open_new_pcap(out_dir: &Path, snaplen: u32, network: u32, seq: u64) -> anyhow::Result<PcapFile> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("create capture dir {}", out_dir.display()))?;

    let now = chrono::Utc::now();
    let name = format!(
        "ntx-{}-{:06}.pcap",
        now.format("%Y%m%d-%H%M%S"),
        seq
    );
    let path = out_dir.join(name);

    let f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("open pcap file {}", path.display()))?;

    let mut w = BufWriter::new(f);

    // Global header.
    // https://wiki.wireshark.org/Development/LibpcapFileFormat
    w.write_all(&0xa1b2c3d4u32.to_le_bytes())?; // magic (little-endian)
    w.write_all(&2u16.to_le_bytes())?; // version major
    w.write_all(&4u16.to_le_bytes())?; // version minor
    w.write_all(&0i32.to_le_bytes())?; // thiszone
    w.write_all(&0u32.to_le_bytes())?; // sigfigs
    w.write_all(&snaplen.to_le_bytes())?;
    w.write_all(&network.to_le_bytes())?;
    w.flush()?;

    Ok(PcapFile {
        w,
        bytes_written: 24,
    })
}

fn to_pcap_timestamp(ts: SystemTime) -> (u32, u32) {
    let dur = ts
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let sec = dur.as_secs().min(u32::MAX as u64) as u32;
    let usec = dur.subsec_micros();
    (sec, usec)
}
