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
mod coded;
mod prbs;
mod probe;
mod rate;
mod run;
mod scenarios;
mod stress;
mod timing;
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
        other => {
            eprintln!("unknown subcommand `{other}`; expected `probe`, `run`, `stress`, `coded`, `sync`, `rate`, or `adapt`");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
