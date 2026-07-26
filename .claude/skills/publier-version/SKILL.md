---
name: publier-version
description: Publier une version de MixFlow — alignement des versions, tag, release.yml, installeur NSIS, signature minisign et mise à jour automatique (tauri-plugin-updater). À utiliser pour toute question de release, de tag, de latest.json, de pubkey ou d'utilisateurs qui ne reçoivent pas la mise à jour.
---

# Publier une version

## Le fichier qui compte n'est pas le tag

Le tag **ne fait que nommer la release**. La version comparée par le client
est celle de `src-tauri/tauri.conf.json`. Un tag `v0.2.0` posé sur une
`tauri.conf.json` restée en `0.1.0` produit une release que personne ne
recevra.

## Procédure

1. **Aligner la version aux trois endroits** — ils doivent être identiques :
   - `package.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
2. Commit (`chore(release): vX.Y.Z`), puis :
   ```bash
   git tag v0.2.0 && git push --tags
   ```
3. `release.yml` construit l'installeur NSIS et crée une release **brouillon**.
4. **Publier la release à la main.** Tant qu'elle est en brouillon, ses
   fichiers ne sont pas servis par `releases/latest/download/…` et l'endpoint
   de mise à jour renvoie **404** — c'est la cause n°1 d'« aucun utilisateur ne
   voit la mise à jour ».

Le workflow accepte le tag quelle que soit sa casse et peut être lancé
manuellement.

## Signature et chaîne de confiance

`latest.json` est produit et signé par `release.yml`, puis servi par
`https://github.com/maximemarc/SonnarAudio/releases/latest/download/latest.json`.

L'app vérifie la signature **minisign** contre `plugins.updater.pubkey`
(`tauri.conf.json`) avant d'écrire quoi que ce soit : un endpoint compromis ne
suffit pas à faire installer un binaire arbitraire.

- La clé **privée** vit uniquement dans les secrets GitHub
  (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Jamais
  dans le dépôt, jamais dans une réponse, jamais dans un log.
- Seule la clé **publique** est committée, dans `tauri.conf.json`.
- **Perdre la clé privée oblige tous les utilisateurs à réinstaller à la
  main** : les mises à jour signées par une nouvelle clé seront rejetées.

## Ne pas retirer le bloc updater

`tauri-plugin-updater` **refuse de démarrer** sans `plugins.updater` valide
(pubkey + endpoints) dans `tauri.conf.json` : il fait paniquer tout `main()`.
Testé. Ce bloc reste en place même en développement.

## Diagnostic

| Symptôme | Cause |
| --- | --- |
| Endpoint de mise à jour en 404 | release restée en brouillon |
| Aucun client ne se met à jour | version de `tauri.conf.json` non bumpée |
| Mise à jour rejetée côté client | signature faite avec une autre clé que la pubkey embarquée |
| L'app panique au démarrage | bloc `plugins.updater` absent ou invalide |
| Build de release qui ne part pas | casse du tag — le workflow la tolère, vérifier plutôt le push des tags |

## Avant de taguer

```powershell
npm run lint && npm run format:check && npm run typecheck && npm run build
```

```powershell
cd src-tauri; cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test
```

Un tag qui échoue en CI laisse une release brouillon vide à nettoyer à la main.
