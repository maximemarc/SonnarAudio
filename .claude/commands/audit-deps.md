---
description: Audit de sécurité des dépendances (npm + cargo), en écartant les faux positifs Linux-only connus
allowed-tools:
  - Bash(npm audit *)
  - Bash(cd src-tauri)
  - Bash(cargo audit *)
  - Bash(cargo tree *)
  - PowerShell(cd src-tauri)
  - PowerShell(cargo audit *)
  - PowerShell(cargo tree *)
  - Read
  - Grep
---

Reproduis le job `audit` de la CI, qui échoue sur toute sévérité **haute** ou
plus, des deux côtés.

```powershell
npm audit --audit-level=high
```

```powershell
cd src-tauri; cargo audit
```

(`cargo audit` ne compare que `Cargo.lock` à la base RustSec — il ne compile
rien, d'où un job CI sur `ubuntu-latest` alors que le crate ne se construit
que sous Windows. `cargo install cargo-audit --locked` s'il manque.)

## Corriger

Côté npm : `npm audit fix` **sans `--force`**. Une montée de majeure imposée
par `--force` casse plus souvent qu'elle ne répare, et sort des bornes semver.

## Faux positifs attendus — ne pas chercher à les corriger

Dependabot analyse `Cargo.lock`, qui contient les dépendances de **toutes** les
plateformes. MixFlow ne se construit que sous Windows : toute la pile
GTK/GDK/glib de Tauri (backend `webkit2gtk`, Linux uniquement — Windows passe
par WebView2) figure dans le lockfile sans jamais être compilée dans le
binaire livré.

Cas emblématique : **RUSTSEC-2024-0429** (`glib` < 0.20, unsoundness dans
`VariantStrIter`). Dependabot signale lui-même ne pas pouvoir le corriger,
Tauri 2 épinglant la génération gtk-rs 0.18.

Vérification à faire avant de classer un avis « not affected » :

```powershell
cd src-tauri
cargo tree --target x86_64-pc-windows-msvc --invert <crate>
cargo tree --target x86_64-unknown-linux-gnu --invert <crate>
```

Si la cible Windows répond « nothing to print », le crate n'est pas dans le
binaire livré : l'avis est classable « not affected » sur GitHub.

Ne durcis **pas** la CI pour faire échouer sur `unmaintained` / `unsound` :
elle serait rouge en permanence pour du code qui n'est même pas compilé.

## Rapport

Pour chaque avis : sévérité, crate/paquet, **est-il réellement dans le binaire
Windows** (avec la sortie `cargo tree` à l'appui), et l'action retenue —
corrigé, à corriger, ou classé « not affected » avec la justification.

$ARGUMENTS
