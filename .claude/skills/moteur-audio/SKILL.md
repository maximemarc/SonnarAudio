---
name: moteur-audio
description: Travailler sur le moteur audio temps réel de MixFlow (src-tauri/src/audio/ — engine.rs, dsp.rs, controls.rs, model.rs). À utiliser dès qu'il s'agit de resampler, ring buffers, servo de dérive, EQ biquad, ducking, gains/mutes, VU-mètres, craquements, clics, latence ou saturation audio.
---

# Moteur audio MixFlow

Domaine canonique : **48 kHz stéréo**. Le moteur vit dans un thread dédié qui
possède les `cpal::Stream` (`!Send`) ; `main.rs` lui parle par un `mpsc` pour
la topologie et par des atomics (`controls.rs`) pour le live.

## La règle qui prime sur tout : le callback audio ne bloque pas

Dans `CaptureState::process` et `RenderState::render`, jamais de `lock()`
bloquant, d'allocation, d'I/O ni de `println!`. Les paramètres partagés se
lisent en **`try_read`** une fois par bloc (bandes EQ, règles de ducking) et
on garde la valeur précédente si le verrou est pris. Les scalaires passent
par des atomics dans `controls.rs`.

## Avant de modifier quoi que ce soit

```powershell
cd src-tauri; cargo test
```

Les tests couvrent resampler, soft-clip, EQ et `sanitize_bands`. Le test
`exact_preserves_phase_across_blocks_when_upsampling` est un **garde-fou de
non-régression** : il tombe si on recasse la phase du resampler.

## Pièges déjà payés (ne pas les réintroduire)

### Phase du resampler

`Resampler` garde **deux** frames d'historique (`prev` / `prev2`). Côté rendu,
la position résiduelle **repart négative** dès que `step < 1` (périphérique de
sortie au-dessus de 48 kHz). La clamper à 0 jette de la phase à chaque bloc :
crachotement périodique audible. Si une modification touche l'accumulateur de
position, relancer le test de phase ci-dessus.

### Servo de dérive

`RenderState::render` applique ±0,3 % de trim au resampler pour garder le
remplissage des rings calé sur `PREFILL_FRAMES` (~30 ms). Il s'asservit sur le
**minimum des rings VIVANTS** : un ring vide depuis plus de
`DEAD_AFTER_EMPTY_BLOCKS` blocs (producteur mort, capture échouée) est exclu.
Sinon son zéro permanent épingle le trim et fait saturer les rings sains du
même bus.

### EQ paramétrique

1 à 10 biquads peaking par ligne (`LineConfig.eq_bands`), freq 20 Hz–20 kHz,
gain ±12 dB, Q = 1, écart minimum `MIN_BAND_FREQ_RATIO` entre points imposé par
`sanitize_bands`. Appliqués dans le callback de capture.

Quand le nombre de points change, les filtres sont **réconciliés par
fréquence**, pas recréés : recréer perd l'état des bandes inchangées et
produit un clic audible. Le champ `eq` (legacy 5 bandes) est migré au
démarrage — ne pas le supprimer sans migration.

Les formules RBJ de `dsp.rs` sont **dupliquées côté frontend** dans
`src/eqMath.ts` pour dessiner la courbe. Toute modification des coefficients
doit être répercutée des deux côtés, sinon la courbe ment sur ce qu'on entend.

### Ducking

Enveloppe calculée par ligne (capture) + règles source→cible lues en `try_read`
dans le rendu.

- Une source **muette ne ducke rien**. Le mute s'applique au rendu alors que
  l'enveloppe est calculée avant : sans ce test explicite, Ctrl+Alt+M coupait
  le micro mais continuait de baisser le jeu.
- `env` n'est décrémentée que par le callback de capture. Un `capture_tick`
  sert de battement de cœur : le thread `mixflow-levels` relâche l'enveloppe
  quand le compteur stagne (flux mort). Sans ça les cibles restent atténuées à
  vie.
- `LineConfig.duck_reactivity` ("douce" | "normale" | "rapide") pilote
  `LineCtl.duck_decay`. C'est une propriété de la ligne **source**, pas de la
  règle — même si l'UI l'affiche à côté de chaque règle.

### Valeurs venues du disque

Tout ce qui vient d'un fichier ou d'un import passe par `sanitize_config`. Un
JSON syntaxiquement valide n'est pas sain : `"gain": 1e300` devient `+inf` en
f32, contamine le lissage de gain du rendu puis l'état des biquads — ligne
muette ou bruyante jusqu'au rebuild. Toute nouvelle valeur numérique
persistée doit être bornée là.

## Symptôme → endroit à regarder

| Symptôme | Piste |
| --- | --- |
| Crachotement périodique régulier | phase du resampler (`step < 1`) |
| Saturation / retard qui monte sur un bus | servo, ring mort non exclu |
| Clic à l'ajout ou au retrait d'un point EQ | réconciliation des biquads par fréquence |
| Une cible reste atténuée pour toujours | heartbeat `capture_tick` du ducking |
| Fader / mute / EQ sans effet | `Controls` échangés hors du verrou `config` — voir la skill `commandes-tauri` |
| Ligne muette ou bruyante après un import | borne manquante dans `sanitize_config` |

## Vérification

```powershell
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

Un changement audible (glitch, latence, niveau) doit être écouté pour de vrai :
`cargo test` ne l'entend pas.
