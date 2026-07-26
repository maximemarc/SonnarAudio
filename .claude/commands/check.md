---
description: Lance toute la batterie de vérifications pré-PR (front + back) et corrige ce qui bloque
allowed-tools:
  - Bash(npm run *)
  - Bash(cd src-tauri)
  - Bash(cargo *)
  - PowerShell(npm run *)
  - PowerShell(cd src-tauri)
  - PowerShell(cargo *)
  - Read
  - Grep
  - Glob
---

Lance la batterie complète de vérifications pré-PR, exactement celle de
`.github/workflows/ci.yml` et de CONTRIBUTING.md.

Prépare le PATH d'abord (les shells n'ont pas toujours cargo/node) :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\nodejs;" + $env:Path
```

**Frontend :**

1. `npm run lint`
2. `npm run format:check`
3. `npm run typecheck`
4. `npm run build`

**Backend** (depuis `src-tauri`) :

5. `cargo fmt --check`
6. `cargo clippy --all-targets -- -D warnings`
7. `cargo test`

Fais tourner les étapes indépendantes en parallèle quand c'est possible.

Ensuite :

- **Corrige** ce qui échoue, en respectant les contraintes de CLAUDE.md
  (rien de cosmétique en plus, pas de refactor opportuniste).
- La CI échoue sur le **moindre warning clippy**, pas seulement les erreurs.
- Un warning `react-refresh/only-export-components` se règle en sortant
  l'export non-composant dans son propre fichier (cf. `pickIcon.tsx`), pas
  avec un `eslint-disable`.
- Ne mets **jamais** un `#[allow(...)]` ou un `eslint-disable` pour faire
  passer un check sans en parler explicitement dans ton rapport.

Termine par un état net : ce qui passe, ce que tu as corrigé, ce qui reste
rouge et pourquoi.

$ARGUMENTS
