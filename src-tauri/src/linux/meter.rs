//! VU-mètres : crêtes réelles des sorties et des applis.
//!
//! Chaque VU est un `pw-record` qui écrit du f32 brut sur stdout :
//! - **sortie** : `--target <sink>` + `stream.capture.sink=true` = moniteur
//!   de cette sortie (ce qui part vers le HDMI, le casque…) ;
//! - **appli** : `--target <serial du flux>` = le flux lui-même. Une sortie
//!   de flux accepte plusieurs liens, la lecture de l'appli n'est pas gênée.
//!   Pour une appli réglée, on mesure la sortie de sa chaîne (après EQ et
//!   volume), donc le VU reflète ce qu'on entend.
//!
//! [`Meters::sync`] démarre/arrête les captures selon la liste voulue ; les
//! crêtes s'accumulent (max) jusqu'au prochain [`Meters::take`].
//!
//! Les nœuds s'appellent `mixflow.meter.<n>` : `Node::is_ours` les écarte
//! de la liste des applis, et aucun nom de chaîne ne peut les heurter (les
//! slugs ne contiennent pas de point).

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use parking_lot::Mutex;

/// 24 kHz suffit largement pour une crête de VU et divise le coût par deux.
const RATE: &str = "24000";
/// Trames lues par crête (20 ms à 24 kHz), en stéréo f32.
const BLOCK_BYTES: usize = 480 * 2 * 4;

/// Ce qu'un VU écoute.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Moniteur d'une sortie (`node.name` du sink).
    Sink(String),
    /// Un flux précis (`object.serial`).
    Stream(String),
}

/// Identifiant d'un VU côté UI : `("outputs", node.name)` ou
/// `("lines", clé d'appli)` — la même forme que l'événement `levels` de
/// Windows, que `levels.ts` lit tel quel.
pub type MeterId = (&'static str, String);

struct Running {
    source: Source,
    child: Child,
}

#[derive(Default)]
pub struct Meters {
    running: Mutex<HashMap<MeterId, Running>>,
    peaks: Arc<Mutex<HashMap<MeterId, f32>>>,
    next: Mutex<u64>,
}

impl Meters {
    /// Aligne les captures en cours sur `wanted` : démarre les nouvelles,
    /// arrête celles qui ne servent plus ou dont la source a changé, et
    /// relance celles dont `pw-record` est mort.
    pub fn sync(&self, wanted: &HashMap<MeterId, Source>) {
        let mut running = self.running.lock();
        running.retain(|id, r| {
            let keep = wanted.get(id) == Some(&r.source) && matches!(r.child.try_wait(), Ok(None));
            if !keep {
                let _ = r.child.kill();
                let _ = r.child.wait();
                self.peaks.lock().remove(id);
            }
            keep
        });
        for (id, source) in wanted {
            if running.contains_key(id) {
                continue;
            }
            let n = {
                let mut n = self.next.lock();
                *n += 1;
                *n
            };
            match self.spawn(id.clone(), source, n) {
                Ok(child) => {
                    running.insert(
                        id.clone(),
                        Running {
                            source: source.clone(),
                            child,
                        },
                    );
                }
                Err(e) => eprintln!("[mixflow] VU {id:?} : {e}"),
            }
        }
        drop(running);
        // Un lecteur arrêté a pu écrire une dernière crête après le retrait.
        self.peaks.lock().retain(|id, _| wanted.contains_key(id));
    }

    /// Crêtes accumulées depuis le dernier appel (remises à zéro).
    pub fn take(&self) -> HashMap<MeterId, f32> {
        let mut p = self.peaks.lock();
        let out = p.clone();
        for v in p.values_mut() {
            *v = 0.0;
        }
        out
    }

    fn spawn(&self, id: MeterId, source: &Source, n: u64) -> Result<Child, String> {
        let (target, capture_sink) = match source {
            Source::Sink(name) => (name.as_str(), true),
            Source::Stream(serial) => (serial.as_str(), false),
        };
        // SPA-JSON : une valeur avec espace ou point doit être citée.
        let props = format!(
            r#"{{ node.name = "mixflow.meter.{n}" node.description = "MixFlow VU" stream.capture.sink = {capture_sink} node.dont-reconnect = true }}"#
        );
        let mut child = Command::new("pw-record")
            .args(["--target", target, "-P", &props])
            .args(["--media-category", "Monitor", "--format", "f32"])
            .args(["--rate", RATE, "--channels", "2", "--latency", "20ms", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("pw-record introuvable : {e}"))?;
        let mut out = child.stdout.take().ok_or("pw-record sans sortie")?;
        let peaks = self.peaks.clone();
        std::thread::Builder::new()
            .name("mixflow-meter".into())
            .spawn(move || {
                let mut buf = vec![0u8; BLOCK_BYTES];
                // Fin de flux = pw-record arrêté (par `sync` ou parce que la
                // cible a disparu) : le prochain `sync` relancera si besoin.
                while out.read_exact(&mut buf).is_ok() {
                    let p = peak(&buf);
                    let mut map = peaks.lock();
                    let e = map.entry(id.clone()).or_insert(0.0);
                    if p > *e {
                        *e = p;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(child)
    }
}

/// Crête absolue d'un bloc de f32 little-endian.
///
/// Par index plutôt que `chunks_exact(4)` / `as_chunks::<4>()` : clippy
/// réclame l'un ou l'autre selon le MSRV (`as_chunks` n'existe qu'en 1.88+).
fn peak(bytes: &[u8]) -> f32 {
    (0..bytes.len() / 4)
        .map(|i| {
            let c = &bytes[i * 4..i * 4 + 4];
            f32::from_le_bytes([c[0], c[1], c[2], c[3]]).abs()
        })
        .filter(|v| v.is_finite())
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_reads_le_f32() {
        let mut b = Vec::new();
        for v in [0.1f32, -0.7, 0.3, f32::NAN] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        assert!((peak(&b) - 0.7).abs() < 1e-6);
        assert_eq!(peak(&[]), 0.0);
    }
}
