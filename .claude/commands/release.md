---
description: Prépare une release — aligne les 3 versions, vérifie tout, tague et rappelle l'étape manuelle
argument-hint: [version, ex. 0.2.0]
disable-model-invocation: true
allowed-tools:
  - Bash(npm run *)
  - Bash(cd src-tauri)
  - Bash(cargo check *)
  - Bash(cargo test *)
  - Bash(cargo fmt *)
  - Bash(cargo clippy *)
  - Bash(git status *)
  - Bash(git diff *)
  - Bash(git log *)
  - Read
  - Grep
  - Glob
---

Prépare la publication de la version **$1**.

Consulte la skill `publier-version` pour le détail de la chaîne de confiance.

## 1. Aligner les trois versions

Elles doivent être **identiques**. Le tag ne fait que nommer la release ; la
version comparée par le client est celle de `tauri.conf.json`.

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

Après avoir touché `Cargo.toml`, mets `Cargo.lock` à jour
(`cd src-tauri; cargo check`).

## 2. Vérifier

Toute la batterie pré-PR doit être verte — un tag qui échoue en CI laisse une
release brouillon vide à nettoyer à la main. Enchaîne `/check`, ou :

```powershell
npm run lint; npm run format:check; npm run typecheck; npm run build
```

```powershell
cd src-tauri; cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test
```

## 3. Contrôles spécifiques release

- Le bloc `plugins.updater` (pubkey + endpoints) est **toujours présent** dans
  `tauri.conf.json` : sans lui, `tauri-plugin-updater` fait paniquer `main()`.
- Aucune clé privée dans le diff. Seule la clé **publique** est committée ; la
  privée vit uniquement dans les secrets GitHub.
- Les secrets `TAURI_SIGNING_PRIVATE_KEY` et
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` doivent exister sur le dépôt, sinon la
  signature échoue et la mise à jour auto est cassée pour cette version.

## 4. Commit et tag

```
chore(release): v$1
```

Puis, **après validation explicite de l'utilisateur** (pousser un tag
déclenche une build publique) :

```bash
git tag v$1 && git push --tags
```

## 5. Rappelle l'étape manuelle

`release.yml` crée une release **brouillon**. Tant qu'elle n'est pas publiée à
la main, ses fichiers ne sont pas servis par `releases/latest/download/…` et
l'endpoint de mise à jour renvoie **404** : personne ne recevra la version.

$ARGUMENTS
