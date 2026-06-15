//! `dov-probe` — emulation harness for the data-over-voice modem.
//!
//! Subcommands:
//!   * `probe` — push tones/chirps through each codec; passband + spectrograms.
//!   * `run`   — end-to-end 8-FSK BER through each codec (M1; the real metric).
//!
//! Run: `cargo run -p dov-harness -- run`  (or `probe`)

mod adapt;
mod ber;
mod bridge;
mod bt;
mod coded;
mod link;
mod prbs;
mod probe;
mod rate;
mod run;
mod scenarios;
mod stress;
mod timing;
mod validate;
mod wav;

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "run".to_string());
    let result = match cmd.as_str() {
        "probe" => probe::run(),
        "run" => run::run(),
        "stress" => stress::run(),
        "coded" => coded::run(),
        "sync" => timing::run(),
        "rate" => rate::run(),
        "adapt" => adapt::run(),
        "validate" => validate::run(),
        "bt" => bt::run(),
        "selftest" => {
            let msg = std::env::args().skip(2).collect::<Vec<_>>().join(" ");
            let msg = if msg.is_empty() {
                "the quick brown fox jumps over the lazy dog 0123456789".to_string()
            } else {
                msg
            };
            link::run_selftest(&msg)
        }
        "send" => {
            let a: Vec<String> = std::env::args().skip(2).collect();
            match a.first() {
                Some(msg) if !msg.is_empty() => link::run_send(msg, a.get(1).cloned()),
                _ => {
                    eprintln!("usage: send \"<message>\" [alsa-device]");
                    std::process::exit(2);
                }
            }
        }
        "recv" => {
            let a: Vec<String> = std::env::args().skip(2).collect();
            let seconds = a.first().and_then(|s| s.parse().ok()).unwrap_or(12.0);
            link::run_recv(seconds, a.get(1).cloned())
        }
        "loopback" => {
            let a: Vec<String> = std::env::args().skip(2).collect();
            link::run_loopback(a.first().cloned(), a.get(1).cloned())
        }
        other => {
            eprintln!("unknown subcommand `{other}`; expected one of: probe run stress coded sync rate adapt validate bt selftest send recv loopback");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
