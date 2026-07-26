---
description: Démarre l'app en dev (nettoie le port 5173, vérifie le PATH et la redirection MAX_PATH)
disable-model-invocation: true
allowed-tools:
  - PowerShell(Get-NetTCPConnection *)
  - Read
---

Démarre MixFlow en développement, en écartant d'abord les trois causes
d'échec habituelles.

**1. PATH** — les shells n'ont pas toujours cargo/node :

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\nodejs;" + $env:Path
```

**2. Port 5173** — vite est en `strictPort`, un zombie node le fait échouer :

```powershell
Get-NetTCPConnection -LocalPort 5173 -State Listen -ErrorAction SilentlyContinue
```

S'il y en a un, montre le PID et le nom du processus, puis **demande
confirmation avant de le tuer** — ce peut être une session de travail de
l'utilisateur.

**3. MAX_PATH** — vérifie que `src-tauri/.cargo/config.toml` existe. S'il
manque, cargo échouera en `LNK1104` : le recréer (le fichier est gitignoré,
il ne viendra jamais du dépôt).

```toml
[build]
target-dir = "C:/un/chemin/court"
```

**Ensuite**, lance le serveur **en arrière-plan** (`run_in_background`) — il
ne rend jamais la main :

```powershell
npm run tauri dev
```

Surveille la sortie jusqu'à ce que vite serve et que cargo ait fini sa
première compilation (elle est longue à froid). Rapporte la première erreur
de compilation s'il y en a une, et ne reste pas à attendre en boucle.

$ARGUMENTS
