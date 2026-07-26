---
description: Diagnostique un problème audio (craquement, clic, latence, saturation, silence) en partant du symptôme
argument-hint: [symptôme entendu]
allowed-tools:
  - Bash(cd src-tauri)
  - Bash(cargo test *)
  - Bash(cargo clippy *)
  - PowerShell(cd src-tauri)
  - PowerShell(cargo test *)
  - Read
  - Grep
  - Glob
---

Diagnostique ce symptôme audio : **$ARGUMENTS**

Charge la skill `moteur-audio` (et `windows-audio` si le symptôme touche une
app, un câble ou un périphérique).

## Méthode

Pars du symptôme, pas du code. Ce projet a déjà payé plusieurs bugs dont la
signature sonore est caractéristique — commence par écarter ceux-là avant
d'ouvrir une piste neuve.

| Symptôme entendu | Première piste |
| --- | --- |
| Crachotement **périodique et régulier** | phase du resampler : la position résiduelle repart négative quand `step < 1` (sortie > 48 kHz) ; la clamper à 0 jette de la phase à chaque bloc |
| Saturation / retard qui **monte** sur un bus | servo de dérive : un ring mort (producteur disparu) épingle le trim s'il n'est pas exclu après `DEAD_AFTER_EMPTY_BLOCKS` |
| **Clic** à l'ajout ou au retrait d'un point EQ | biquads recréés au lieu d'être réconciliés par fréquence |
| Une cible **reste atténuée** indéfiniment | heartbeat `capture_tick` du ducking : `env` n'est décrémentée que par la capture |
| Le mute du micro **baisse quand même** le jeu | une source muette doit ducker à zéro (le mute s'applique au rendu, l'enveloppe est calculée avant) |
| Fader / mute / EQ **sans effet** | `Controls` échangés hors du verrou `config` (rebuilds concurrents) |
| Ligne **muette ou bruyante** après un import | valeur non bornée dans `sanitize_config` (`1e300` → `+inf` en f32) |
| App routée mais **aucun son** | `cable_render_id` fantôme non revalidé, ou câble apparié par nom complet |
| **Rien** ne sort après un changement de périphérique | `device_warnings` (thread `mixflow-health`, 3 s) ; `stream_err` ne fait qu'un `eprintln!` |
| UI qui **se fige** quelques secondes | commande synchrone faisant du COM, ou COM sous le verrou `config` |

## Ensuite

1. Localise le code concerné et **lis-le** avant de conclure.
2. Vérifie que le comportement observé découle bien de ce code — n'accuse pas
   une piste du tableau sans l'avoir confirmée dans les sources.
3. `cd src-tauri; cargo test` : les tests couvrent resampler, soft-clip, EQ et
   `sanitize_bands`. Si `exact_preserves_phase_across_blocks_when_upsampling`
   tombe, la phase du resampler est en cause.
4. Si tu corriges : ajoute un test de non-régression quand le bug est
   déterministe et testable hors périphérique réel.

Dis clairement ce que tu as **vérifié dans le code** et ce qui reste à
confirmer **à l'oreille** — `cargo test` n'entend pas un craquement.
