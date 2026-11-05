//! Modèle persistant de MixFlow sous Linux — sérialisé en JSON
//! (`~/.config/com.mixflow.app/mixflow-linux.json`) et reflété côté
//! TypeScript dans `src/linux/types.ts`.
//!
//! Une **application** est identifiée par une clé stable : le nom du binaire
//! (`application.process.binary`) quand PipeWire le connaît, sinon
//! `application.name`. Chaque appli configurée possède sa propre chaîne de
//! filtres PipeWire (volume + EQ), identifiée par `slug`.

use serde::{Deserialize, Serialize};

/// Nombre maximal de points d'EQ — la chaîne PipeWire en contient toujours
/// exactement autant (les points inutilisés restent à 0 dB, donc neutres),
/// ce qui permet d'ajouter/retirer un point à chaud, sans redémarrage.
pub const MAX_BANDS: usize = 10;
pub const MAX_GAIN: f32 = 1.5;
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Couleurs d'accent attribuées aux applis, dans l'ordre d'arrivée (mêmes
/// teintes que `LINE_COLORS` côté frontend).
const COLORS: [&str; 8] = [
    "#7c3aed", "#22d3ee", "#f59e0b", "#10b981", "#ef4444", "#ec4899", "#3b82f6", "#a3e635",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EqBand {
    pub freq: f32,
    pub gain: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    pub bands: Vec<EqBand>,
}

/// Réglages d'une application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    /// Clé d'identification (binaire ou nom d'appli), comparée sans casse.
    pub key: String,
    /// Nom affiché.
    pub name: String,
    /// Identifiant de la chaîne : `[a-z0-9-]`, unique. Sert au nom du nœud
    /// PipeWire (`mixflow.<slug>`), du fichier de conf et de l'instance
    /// systemd (`mixflow-chain@<slug>.service`).
    pub slug: String,
    #[serde(default = "default_color")]
    pub color: String,
    /// Volume linéaire [0 .. 1.5].
    #[serde(default = "default_gain")]
    pub gain: f32,
    #[serde(default)]
    pub muted: bool,
    /// `node.name` de la sortie physique ; vide = sortie par défaut du système.
    #[serde(default)]
    pub output: String,
    #[serde(default = "default_bands")]
    pub eq_bands: Vec<EqBand>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinuxConfig {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub apps: Vec<AppSettings>,
    /// Préréglages d'EQ de l'utilisateur (ceux d'usine vivent côté frontend).
    #[serde(default)]
    pub eq_presets: Vec<EqPreset>,
}

impl Default for LinuxConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            apps: Vec::new(),
            eq_presets: Vec::new(),
        }
    }
}

fn default_gain() -> f32 {
    1.0
}

fn default_color() -> String {
    COLORS[0].to_string()
}

/// Courbe neutre : les 5 bandes classiques à 0 dB (= `DEFAULT_EQ_BANDS`).
pub fn default_bands() -> Vec<EqBand> {
    [80.0, 250.0, 1000.0, 4000.0, 12000.0]
        .iter()
        .map(|&freq| EqBand { freq, gain: 0.0 })
        .collect()
}

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

/// Borne et trie une courbe d'EQ : 1..=10 points, 20 Hz–20 kHz, ±12 dB.
pub fn sanitize_bands(bands: Vec<EqBand>) -> Vec<EqBand> {
    let mut out: Vec<EqBand> = bands
        .into_iter()
        .map(|b| EqBand {
            freq: finite_or(b.freq, 1000.0).clamp(20.0, 20000.0),
            gain: finite_or(b.gain, 0.0).clamp(-12.0, 12.0),
        })
        .collect();
    out.sort_by(|a, b| a.freq.total_cmp(&b.freq));
    out.truncate(MAX_BANDS);
    if out.is_empty() {
        out = default_bands();
    }
    out
}

pub fn sanitize_gain(g: f32) -> f32 {
    finite_or(g, 1.0).clamp(0.0, MAX_GAIN)
}

/// Transforme une clé d'appli en identifiant sûr pour un nom de nœud, de
/// fichier et d'instance systemd.
pub fn slugify(key: &str) -> String {
    let mut s = String::new();
    for c in key.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s: String = s.trim_matches('-').chars().take(40).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "app".into()
    } else {
        s
    }
}

impl LinuxConfig {
    pub fn find(&self, key: &str) -> Option<&AppSettings> {
        self.apps.iter().find(|a| a.key.eq_ignore_ascii_case(key))
    }

    pub fn find_mut(&mut self, key: &str) -> Option<&mut AppSettings> {
        self.apps
            .iter_mut()
            .find(|a| a.key.eq_ignore_ascii_case(key))
    }

    /// Slug libre dérivé de `key` (suffixe `-2`, `-3`… en cas de collision).
    pub fn unique_slug(&self, key: &str) -> String {
        let base = slugify(key);
        let mut slug = base.clone();
        let mut n = 2;
        while self.apps.iter().any(|a| a.slug == slug) {
            slug = format!("{base}-{n}");
            n += 1;
        }
        slug
    }

    /// Ajoute une appli avec des réglages neutres et renvoie son index.
    pub fn add_app(&mut self, key: &str, name: &str) -> usize {
        let slug = self.unique_slug(key);
        let color = COLORS[self.apps.len() % COLORS.len()].to_string();
        self.apps.push(AppSettings {
            key: key.to_string(),
            name: name.to_string(),
            slug,
            color,
            gain: 1.0,
            muted: false,
            output: String::new(),
            eq_bands: default_bands(),
        });
        self.apps.len() - 1
    }

    /// Tout ce qui vient du disque passe ici : un JSON valide n'est pas
    /// forcément sain (gains infinis, slugs dupliqués ou dangereux…).
    pub fn sanitize(&mut self) {
        let mut seen_keys: Vec<String> = Vec::new();
        let mut seen_slugs: Vec<String> = Vec::new();
        let apps = std::mem::take(&mut self.apps);
        for mut a in apps {
            if a.key.trim().is_empty() || seen_keys.contains(&a.key.to_lowercase()) {
                continue;
            }
            seen_keys.push(a.key.to_lowercase());
            let mut slug = slugify(&a.slug);
            let base = slug.clone();
            let mut n = 2;
            while seen_slugs.contains(&slug) {
                slug = format!("{base}-{n}");
                n += 1;
            }
            seen_slugs.push(slug.clone());
            a.slug = slug;
            a.gain = sanitize_gain(a.gain);
            a.eq_bands = sanitize_bands(a.eq_bands);
            if a.name.trim().is_empty() {
                a.name = a.key.clone();
            }
            self.apps.push(a);
        }
        for p in &mut self.eq_presets {
            p.bands = sanitize_bands(std::mem::take(&mut p.bands));
        }
        self.eq_presets.retain(|p| !p.name.trim().is_empty());
        self.schema_version = CURRENT_SCHEMA_VERSION;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_is_safe() {
        assert_eq!(slugify("Firefox-ESR"), "firefox-esr");
        assert_eq!(slugify("eldenring.exe"), "eldenring-exe");
        assert_eq!(slugify("  ../../etc  "), "etc");
        assert_eq!(slugify("€€€"), "app");
    }

    #[test]
    fn unique_slugs() {
        let mut c = LinuxConfig::default();
        c.add_app("firefox", "Firefox");
        c.add_app("Firefox!", "Firefox bis");
        assert_eq!(c.apps[1].slug, "firefox-2");
    }

    #[test]
    fn sanitize_clamps_and_dedupes() {
        let mut c = LinuxConfig::default();
        c.add_app("a", "A");
        c.add_app("A", "A bis");
        c.apps[0].gain = f32::INFINITY;
        c.apps[0].eq_bands = vec![
            EqBand {
                freq: 1e9,
                gain: 99.0,
            },
            EqBand {
                freq: 5.0,
                gain: f32::NAN,
            },
        ];
        c.sanitize();
        assert_eq!(c.apps.len(), 1);
        assert_eq!(c.apps[0].gain, 1.0);
        assert_eq!(
            c.apps[0].eq_bands[0],
            EqBand {
                freq: 20.0,
                gain: 0.0
            }
        );
        assert_eq!(
            c.apps[0].eq_bands[1],
            EqBand {
                freq: 20000.0,
                gain: 12.0
            }
        );
    }

    #[test]
    fn sanitize_bands_limits_count() {
        let many = (0..15)
            .map(|i| EqBand {
                freq: 100.0 + i as f32 * 100.0,
                gain: 1.0,
            })
            .collect();
        assert_eq!(sanitize_bands(many).len(), MAX_BANDS);
        assert_eq!(sanitize_bands(vec![]), default_bands());
    }
}
