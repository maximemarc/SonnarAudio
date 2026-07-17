# MixFlow

Application desktop de **routage et mixage audio virtuel**, inspirée de SteelSeries Sonar : canaux virtuels (Game / Chat / Media…) sélectionnables comme haut-parleurs dans Windows, routage d'applications par glisser-déposer, égaliseur paramétrique par canal, sorties multiples simultanées (casque + haut-parleurs), et **priorisation par ducking** ("quand Chat parle, baisse Game de 50 %").

![stack](https://img.shields.io/badge/stack-Tauri%202%20·%20Rust%20·%20React-7c3aed)
![ci](https://img.shields.io/badge/CI-GitHub%20Actions-2088FF)

---

## 1. Architecture & choix techniques

### Pourquoi Tauri 2 + Rust + cpal ?

| Critère              | Justification                                                                                                                                                                                                                            |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Audio temps réel** | Le mixage se fait dans des callbacks audio (~10 ms). Rust garantit zéro GC-pause et un accès direct à **WASAPI** via [`cpal`](https://crates.io/crates/cpal) — Electron/Python introduisent des latences et du jitter inacceptables ici. |
| **Poids & perfs**    | Binaire ~10 Mo, RAM ~60 Mo (WebView2 système), vs. ~150 Mo+ pour Electron.                                                                                                                                                               |
| **UI moderne**       | Frontend React + Vite : itération rapide, design épuré en CSS pur.                                                                                                                                                                       |
| **Sécurité**         | IPC typé via commandes Tauri, pas de Node intégré.                                                                                                                                                                                       |

### Flux de données

```
┌────────────── UI (React) ──────────────┐
│  faders · matrice · règles de ducking  │
└──────┬──────────────────────▲──────────┘
       │ invoke (commandes)   │ events "levels" (20 Hz) + "engine_status"
┌──────▼──────────────────────┴──────────┐
│      main.rs — état + persistance      │   config JSON → %AppData%/com.mixflow.app/
│  • cmd topologie  → rebuild du moteur  │
│  • cmd live (gain/mute/duck) → atomics │   (aucun glitch, pas de rebuild)
└──────┬─────────────────────────────────┘
       │ mpsc
┌──────▼─────────────────────────────────┐
│   Thread moteur (possède les Streams)  │
│                                        │
│ capture (mic / CABLE Output)           │
│   └─ downmix stéréo → resample 48 kHz  │
│      └─ ring buffer lock-free / route  │
│         └─ Σ (gain ligne × gain route  │
│              × facteur de ducking)     │
│            └─ resample → périph. sortie│
│               └─ soft-clip → WASAPI    │
└────────────────────────────────────────┘
```

Principes clés :

- **Domaine canonique 48 kHz stéréo** : chaque ligne y est convertie (rééchantillonneur linéaire streaming), le mix s'y fait, puis conversion vers la fréquence du périphérique de sortie.
- **Topologie immuable** : ajouter/supprimer une ligne, changer un périphérique ou une route **reconstruit** tous les streams (coupure < 100 ms, acceptable pour un changement structurel).
- **Paramètres continus lock-free** : gains, mutes et montants de ducking sont des `AtomicF32`/`AtomicBool` lus à chaque bloc audio → les faders sont fluides, **sans glitch ni rebuild**.
- **Ducking (priorisation)** : chaque ligne alimente un suiveur d'enveloppe ; les règles `source → cible` atténuent la cible proportionnellement à l'activité de la source (gate ~-40 dBFS, lissage 10 ms).
- **VU-mètres** : pics écrits par les callbacks dans des atomics, événement Tauri à 20 Hz, rendu via `requestAnimationFrame` **hors React** (zéro re-render).
- **Persistance** : `mixflow.config.json` sauvegardé à chaque modification, rechargé au démarrage.

### À propos des "câbles virtuels"

Créer un _endpoint_ audio Windows de toutes pièces exige un **driver noyau signé** (comme le font SteelSeries ou VB-Audio). MixFlow adopte l'approche standard des apps grand public : il **s'appuie sur VB-Cable** (gratuit) pour les endpoints virtuels, et fournit toute la partie routage / mixage / priorisation par-dessus. Voir §3.

---

## 2. Structure du projet

```
mixflow/
├── package.json / vite.config.ts / tsconfig.json / index.html
├── eslint.config.js / .prettierrc.json / commitlint.config.js
├── .husky/                       # hooks Git (pre-commit, commit-msg)
├── .github/workflows/            # CI (ci.yml) + release (release.yml)
├── src/                          # Frontend React
│   ├── main.tsx · App.tsx · styles.css
│   ├── types.ts                  # miroir TS du modèle Rust
│   ├── api.ts                    # wrappers typés des commandes Tauri
│   ├── levels.ts                 # store VU hors-React
│   ├── eqMath.ts                 # maths de courbe EQ (miroir de dsp.rs)
│   └── components/
│       ├── ChannelStrip.tsx · MasterStrip.tsx · ChatMix.tsx
│       ├── EqPage.tsx · DeviceSelect.tsx · VSlider.tsx
│       └── DuckingPanel.tsx · ProfilesPanel.tsx · Icons.tsx · pickIcon.tsx
└── src-tauri/                    # Backend Rust
    ├── Cargo.toml · rustfmt.toml · tauri.conf.json · build.rs
    ├── capabilities/default.json
    ├── icons/
    └── src/
        ├── main.rs               # état, commandes, persistance, événements
        ├── winapps.rs            # sessions WASAPI + routage par app (COM)
        └── audio/
            ├── model.rs          # config sérialisable (lignes, routes, EQ, ducking)
            ├── dsp.rs            # AtomicF32, resampler, biquad EQ, soft-clip (+ tests)
            ├── controls.rs       # plan de contrôle partagé (atomics)
            └── engine.rs         # moteur temps réel cpal/WASAPI
```

---

## 3. Installation — pas à pas (Windows)

### Étape A — Prérequis de compilation

1. **Visual Studio Build Tools** (compilateur C++ requis par Rust) :
   télécharger sur <https://visualstudio.microsoft.com/fr/visual-cpp-build-tools/> et cocher **« Développement Desktop en C++ »**.
2. **Rust** : installer via <https://rustup.rs> (garder les choix par défaut, toolchain MSVC).
3. **Node.js LTS** (≥ 20) : <https://nodejs.org>.
4. **WebView2** : déjà présent sur Windows 11 (sinon : <https://developer.microsoft.com/microsoft-edge/webview2/>).

> Vérification : ouvrir un **nouveau** terminal et lancer `cargo --version`, `node --version`.

### Étape B — Câble virtuel (pour capturer l'audio des applications)

1. Installer **VB-Cable** : <https://vb-audio.com/Cable/> (dézipper → clic droit sur `VBCABLE_Setup_x64.exe` → _Exécuter en tant qu'administrateur_ → redémarrer).
2. Cela crée deux endpoints :
   - **CABLE Input** (sortie) → à définir comme périphérique de sortie de vos jeux/apps ;
   - **CABLE Output** (entrée) → ce que MixFlow capture.
3. Pour plusieurs lignes simultanées (Game + Media…), installer aussi **VB-Cable A+B** (donation) ou utiliser les câbles de Voicemeeter.

### Étape C — Compiler & lancer

```powershell
cd mixflow
npm install          # dépendances frontend + CLI Tauri
npm run tauri dev    # mode développement (1re compilation Rust : 5-10 min)
```

Build de production (installeur NSIS dans `src-tauri/target/release/bundle/`) :

```powershell
npm run tauri build
```

### Étape D — Premier routage (exemple Sonar classique)

Au premier lancement, MixFlow s'approprie automatiquement les câbles libres
et **renomme leur côté "rendu" au nom du canal** — aucune manipulation de
`CABLE Input`/`Output` à faire à la main :

1. Dans MixFlow, section **Sortie** de chaque canal (Game, Chat, Media) :
   choisissez votre casque ou vos haut-parleurs physiques (une ligne peut
   même cocher plusieurs sorties à la fois — casque **et** enceintes).
2. Glissez une application détectée (panneau **Applications**) sur le canal
   voulu — ou utilisez le menu déroulant de l'app. Son audio bascule
   immédiatement, et Windows retient l'association pour les prochains
   lancements de cette app.
3. Le panneau **Priorité** contient déjà « quand Chat est actif, baisser
   Game de 50 % ». Onglet **Égaliseur** pour sculpter chaque canal (glisser
   les points sur la courbe, ou saisir freq/gain au clavier).
4. Bougez les faders : tout est appliqué en direct, et sauvegardé
   automatiquement (persistance débouncée, ~800 ms).

---

## 4. Fonctionnalités

- ✅ Canaux virtuels auto-liés à un câble VB-Audio, renommés et
  sélectionnables comme haut-parleurs directement dans Windows
- ✅ Détection des applications qui jouent du son + routage par
  glisser-déposer (API `IAudioPolicyConfigFactory`, persisté par Windows)
- ✅ **Sorties multiples simultanées** par canal (fan-out — casque +
  haut-parleurs en même temps)
- ✅ **Égaliseur paramétrique** (1 à 10 points par canal, glisser sur la
  courbe ou saisie numérique freq/gain), presets d'usine + custom
- ✅ Priorisation par ducking sidechain (règles source → cible, montant réglable)
- ✅ Fader MASTER global + faders par canal (0–150 %), mute partout
- ✅ VU-mètres 20 Hz + halo "signal actif" sur chaque tranche
- ✅ Mix multi-fréquences (44,1 / 48 / 96 kHz mélangés sans problème)
- ✅ Servo de dérive d'horloge (anti-craquement) + soft-clip anti-saturation
- ✅ Persistance JSON débouncée (garde-fous : écart mini entre points EQ,
  `schema_version` pour les futures migrations)
- ✅ Réduction dans le tray (le mix continue en arrière-plan)
- ✅ Avertissements en direct (périphérique manquant, stream en échec, et
  débranchement détecté en cours de session)
- ✅ Démarrage automatique avec Windows
- ✅ Gain indépendant par sortie (équilibre casque/enceintes ou mix
  perso/stream, dès 2 sorties simultanées sur un canal)
- ✅ Icônes réelles des applications détectées
- ✅ Export / import de la configuration (fichier JSON, dialogues natifs)
- ✅ Profils de mix sauvegardés, avec auto-application quand un jeu/app
  choisi passe au premier plan
- ✅ Raccourci global Ctrl+Alt+M : coupe/réactive tous les micros, même
  fenêtre réduite dans le tray
- ✅ Mode Streamer : envoie le mix vers une sortie dédiée en un clic
- ✅ Réactivité du ducking réglable (douce / normale / rapide) par canal
- ⏳ Mise à jour automatique : câblage prêt côté client, en attente d'un
  remote GitHub avec releases + clé de signature

## 5. Tests & qualité

```powershell
cd src-tauri
cargo test                              # tests unitaires DSP + config (10 tests)
cargo fmt --check                       # formatage
cargo clippy --all-targets -- -D warnings  # lint, zéro warning toléré

cd ..
npm run typecheck                       # tsc --noEmit
npm run lint                            # eslint
npm run format:check                    # prettier --check
```

Ces commandes tournent automatiquement en CI sur chaque pull request — voir
[CONTRIBUTING.md](CONTRIBUTING.md) pour le détail des hooks Git (husky) et
du format des commits (Conventional Commits, imposé par commitlint).
