//! Shared impairment scenarios used by `stress` (raw) and `coded` (post-FEC).

use dov_channel::{Agc, ChannelConfig, Erasure};

pub type CfgBuilder = Box<dyn Fn() -> ChannelConfig>;

pub struct Scenario {
    pub name: &'static str,
    /// Whether AMR DTX/VAD is enabled (ignored by GSM).
    pub dtx: bool,
    pub build: CfgBuilder,
}

pub fn all() -> Vec<Scenario> {
    let s = |name, dtx, build| Scenario { name, dtx, build };
    vec![
        s("clean (baseline)", false, Box::new(ChannelConfig::clean)),
        s("DTX / VAD on", true, Box::new(ChannelConfig::clean)),
        s(
            "erasure 3% iid",
            false,
            Box::new(|| ChannelConfig {
                erasure: Some(Erasure::bernoulli(0.03)),
                ..ChannelConfig::clean()
            }),
        ),
        s(
            "erasure 10% burst",
            false,
            Box::new(|| ChannelConfig {
                erasure: Some(Erasure::bursty(0.10, 4.0)),
                ..ChannelConfig::clean()
            }),
        ),
        s(
            "AGC (target 6000)",
            false,
            Box::new(|| ChannelConfig {
                agc: Some(Agc::new(6000.0)),
                ..ChannelConfig::clean()
            }),
        ),
        s(
            "clock drift +50ppm",
            false,
            Box::new(|| ChannelConfig {
                clock_ppm: 50.0,
                ..ChannelConfig::clean()
            }),
        ),
        s(
            "AWGN 20 dB",
            false,
            Box::new(|| ChannelConfig {
                awgn_snr_db: Some(20.0),
                ..ChannelConfig::clean()
            }),
        ),
        s(
            "combo (DTX+3%burst+AGC+20ppm)",
            true,
            Box::new(|| ChannelConfig {
                erasure: Some(Erasure::bursty(0.03, 4.0)),
                agc: Some(Agc::new(6000.0)),
                clock_ppm: 20.0,
                ..ChannelConfig::clean()
            }),
        ),
    ]
}
