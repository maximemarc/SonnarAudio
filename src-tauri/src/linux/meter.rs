//! VU du son général : crête du moniteur de la sortie par défaut.
//!
//! `pw-record` avec `stream.capture.sink=true` et sans cible écoute le
//! moniteur de la sortie PAR DÉFAUT, et WirePlumber le déplace tout seul
//! quand la sortie par défaut change. On lit son flux brut (f32 stéréo
//! 48 kHz) sur stdout et on publie une crête toutes les ~20 ms.
//!
//! Le nœud s'appelle `mixflow.meter.vu` : `Node::is_ours` l'écarte de la
//! liste des applis, et aucun nom de chaîne ne peut le heurter (les slugs
//! ne contiennent pas de point).

use std::io::Read;
use std::process::{Command, Stdio};

/// Trames lues par crête (20 ms à 48 kHz).
const FRAMES: usize = 960;
const CHANNELS: usize = 2;

/// Lance la capture et appelle `on_peak` (crête linéaire 0..1+) à chaque
/// bloc. Rend la main quand `pw-record` s'arrête : l'appelant relance.
pub fn run(mut on_peak: impl FnMut(f32)) -> Result<(), String> {
    let mut child = Command::new("pw-record")
        .args([
            // SPA-JSON : une valeur avec espace ou point doit être citée.
            "-P",
            r#"{ stream.capture.sink = true node.name = "mixflow.meter.vu" node.description = "MixFlow VU" }"#,
            "--media-category",
            "Monitor",
            "--format",
            "f32",
            "--rate",
            "48000",
            "--channels",
            "2",
            "--latency",
            "960",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("pw-record introuvable : {e}"))?;
    let mut out = child.stdout.take().ok_or("pw-record sans sortie")?;
    let mut buf = vec![0u8; FRAMES * CHANNELS * 4];
    let mut blocks = 0u64;
    while out.read_exact(&mut buf).is_ok() {
        on_peak(peak(&buf));
        blocks += 1;
    }
    let _ = child.kill();
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let _ = child.wait();
    if blocks == 0 {
        // Mort avant le premier bloc : c'est une vraie erreur, on la remonte.
        let first = err.lines().next().unwrap_or("arrêt immédiat");
        return Err(format!("pw-record : {first}"));
    }
    Ok(())
}

/// Crête absolue d'un bloc de f32 little-endian.
fn peak(bytes: &[u8]) -> f32 {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]).abs())
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
