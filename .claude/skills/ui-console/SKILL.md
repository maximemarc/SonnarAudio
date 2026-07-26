---
name: ui-console
description: Travailler sur l'interface React de MixFlow (src/ — App.tsx, components/, levels.ts, eqMath.ts, styles.css) — console façon Sonar, tranches de canal, faders à VU, page Égaliseur, drag & drop d'apps, métering, tray. À utiliser pour toute modification visuelle ou d'interaction du frontend.
---

# UI MixFlow (console façon Sonar)

Interface **en français**. Deux onglets, pilotés par le state `view` dans
`App.tsx` :

- **Console** — MASTER (fader global + zone « À router » des apps non
  routées) | Canaux (Game / Chat / Media : sortie, EQ mini, fader à VU intégré,
  zone Apps drop-target) + ChatMix | Micro. Ducking dessous.
- **Égaliseur** (`EqPage.tsx`) — canal sélectionnable, courbe SVG interactive.

## Performance du métering : hors de React

Tout le métering passe par rAF **en dehors de React** (`levels.ts`). Les
événements `levels` arrivent à 20 Hz. Ne jamais les router dans un `useState` :
ça re-rend la console entière 20 fois par seconde.

- VU dans la piste du fader : `VSlider` prop `meter`, id `"*"` = agrégat.
- Halo « du son passe ici » : `useLiveLevel` pilote la variable CSS `--live`
  (opacité).

## Saisie continue : état local + appel throttlé

Motif obligatoire dès qu'un geste produit plus d'événements que le backend ne
peut en absorber.

Sur la courbe EQ, pendant le drag d'un point, `dragBands` (état local) donne le
retour visuel **instantané** pendant que l'appel réseau reste throttlé à 60 ms.
Sans ça, un drag rapide saccade ou se perd : le round-trip backend est plus
lent que les événements pointeur.

Même famille de motif pour la saisie numérique (`EqPointChip`) et le nom de
canal : **tampon local, commit au blur ou à Enter**.

## Courbe EQ

- Interactions : drag d'un point = freq + gain, double-clic = ajout, clic droit
  = suppression.
- Les maths sont dans `src/eqMath.ts`, avec les **mêmes formules RBJ que
  `src-tauri/src/audio/dsp.rs`**. Toucher aux coefficients d'un côté sans
  l'autre fait mentir la courbe sur ce qu'on entend.
- Presets d'usine `BUILTIN_EQ_PRESETS`, et `BUILTIN_MIC_EQ_PRESETS` pour les
  micros. Presets custom persistés dans `AppConfig.eq_presets`
  (`set_line_eq_bands` / `save_eq_preset` / `delete_eq_preset`).
- Les tranches montrent une mini-courbe cliquable qui ouvre l'onglet.

## Drag & drop des apps

Fonctionne **uniquement** parce que `"dragDropEnabled": false` est posé sur la
fenêtre dans `tauri.conf.json` : sinon la WebView Tauri intercepte le drop
avant le HTML. Ne pas retirer ce réglage.

## Découpage des fichiers (contrainte de lint)

`pickIcon` vit dans son propre fichier (`pickIcon.tsx`, pas `Icons.tsx`) pour
éviter l'avertissement `react-refresh/only-export-components`. **Tout nouvel
export non-composant depuis un module de composants doit suivre ce
découpage** — la CI échoue sur le moindre warning ESLint.

## Synchronisation avec le backend

`App.tsx` écoute `config_updated` (payload `AppConfig`) : c'est ce qui garde
l'UI juste quand l'état change sans passer par elle (raccourci global
Ctrl+Alt+M, auto-switch de profil). Ne pas supposer que l'UI est la seule
source de vérité.

Les avertissements `engine_status` et `device_warnings: string[]` sont mergés
dans le **même `warn-banner`**.

Fermer la fenêtre **réduit dans le tray** (menu Ouvrir / Quitter), ça ne quitte
pas l'app.

## Vérification

```powershell
npm run lint
npm run format:check
npm run typecheck
npm run build
```

Un changement visuel se regarde pour de vrai — `npm run tauri dev` (en
arrière-plan ; penser aux zombies sur le port 5173, vite est en `strictPort`).
