//! The "control plane" shared between the UI (Tauri commands), the levels
//! emitter and the real-time audio callbacks.
//!
//! Everything the audio thread reads on every block is either an atomic
//! (gains, mutes, meters) or behind a `try_read` (ducking rules), so the
//! callbacks never block on the UI. The structure itself is immutable: any
//! topology change (line/output/route/device added or removed) creates a new
//! `Controls` and rebuilds the engine.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::RwLock;

use super::dsp::AtomicF32;
use super::model::{AppConfig, DuckRule, EqBandCfg};

/// Live parameters of one route (line -> output).
pub struct RouteCtl {
    /// Per-route gain, linear [0.0 .. 1.5].
    pub gain: AtomicF32,
}

/// Live parameters + meters of one virtual line.
pub struct LineCtl {
    /// Line fader, linear [0.0 .. 1.5].
    pub gain: AtomicF32,
    pub muted: AtomicBool,
    /// Block peak written by the capture callback, read-and-decayed by the
    /// levels emitter (VU meter).
    pub peak: AtomicF32,
    /// Decaying envelope follower — the ducking side-chain signal.
    pub env: AtomicF32,
    /// Parametric EQ bands — written by the UI, `try_read` each block by
    /// the capture callback, which rebuilds only the coefficients that
    /// actually moved.
    pub eq: RwLock<Vec<EqBandCfg>>,
    /// output_id -> route gain.
    pub routes: HashMap<String, Arc<RouteCtl>>,
}

/// Live parameters + meter of one output bus.
pub struct OutputCtl {
    pub gain: AtomicF32,
    pub muted: AtomicBool,
    pub peak: AtomicF32,
}

pub struct Controls {
    /// line_id -> controls.
    pub lines: HashMap<String, Arc<LineCtl>>,
    /// output_id -> controls.
    pub outputs: HashMap<String, Arc<OutputCtl>>,
    /// Ducking rules; written by the UI, `try_read` by render callbacks.
    pub ducking: RwLock<Vec<DuckRule>>,
    /// Global MASTER, multiplied into every output bus.
    pub master: AtomicF32,
}

impl Controls {
    /// Snapshot a config into a fresh control plane. Every line/output gets
    /// an entry even when dormant (no device), so faders and meters always
    /// have somewhere to live.
    pub fn from_config(cfg: &AppConfig) -> Arc<Self> {
        let lines = cfg
            .lines
            .iter()
            .map(|l| {
                let routes = l
                    .routes
                    .iter()
                    .map(|r| {
                        (
                            r.output_id.clone(),
                            Arc::new(RouteCtl {
                                gain: AtomicF32::new(r.gain),
                            }),
                        )
                    })
                    .collect();
                (
                    l.id.clone(),
                    Arc::new(LineCtl {
                        gain: AtomicF32::new(l.gain),
                        muted: AtomicBool::new(l.muted),
                        peak: AtomicF32::new(0.0),
                        env: AtomicF32::new(0.0),
                        eq: RwLock::new(l.eq_bands.clone()),
                        routes,
                    }),
                )
            })
            .collect();

        let outputs = cfg
            .outputs
            .iter()
            .map(|o| {
                (
                    o.id.clone(),
                    Arc::new(OutputCtl {
                        gain: AtomicF32::new(o.gain),
                        muted: AtomicBool::new(o.muted),
                        peak: AtomicF32::new(0.0),
                    }),
                )
            })
            .collect();

        Arc::new(Self {
            lines,
            outputs,
            ducking: RwLock::new(cfg.ducking.clone()),
            master: AtomicF32::new(cfg.master_gain),
        })
    }
}
