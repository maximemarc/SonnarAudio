# SessionStart — état de l'environnement local, injecté dans le contexte via
# hookSpecificOutput.additionalContext. On ne signale que ce qui est ANORMAL :
# le reste est déjà dans CLAUDE.md, le répéter coûte du contexte pour rien.

. "$PSScriptRoot\_common.ps1"

$null = Read-HookInput
$root = Get-ProjectDir
$notes = @()

Initialize-MixFlowPath

# 1. Redirection MAX_PATH : sans elle, cargo échoue en LNK1104 ici.
$cargoCfg = Join-Path $root 'src-tauri\.cargo\config.toml'
if (-not (Test-Path $cargoCfg)) {
    $notes += 'src-tauri/.cargo/config.toml absent : le chemin de ce dossier est long, cargo peut echouer en LNK1104 (MAX_PATH). Le recreer avec [build] target-dir = "C:/un/chemin/court" — fichier gitignore, voir CLAUDE.md.'
}

# 2. Toolchain : sans cargo/node dans le PATH, toutes les commandes du projet
#    echouent avec un message peu parlant.
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $notes += 'cargo introuvable dans le PATH, meme apres ajout de ~/.cargo/bin : les commandes backend echoueront.'
}
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    $notes += 'node/npm introuvables dans le PATH : lint, typecheck, build et les hooks husky ne peuvent pas tourner.'
}
if (-not (Test-Path (Join-Path $root 'node_modules'))) {
    $notes += "node_modules absent : lancer 'npm install' (installe aussi les hooks husky)."
}

# 3. Zombie sur 5173 : vite est en strictPort, il refusera de demarrer.
try {
    $busy = Get-NetTCPConnection -LocalPort 5173 -State Listen -ErrorAction SilentlyContinue
    if ($busy) {
        $procIds = ($busy | Select-Object -ExpandProperty OwningProcess -Unique) -join ', '
        $notes += "Le port 5173 est deja occupe (PID $procIds). Vite est en strictPort : tuer le zombie avant 'npm run tauri dev'."
    }
} catch { }

if ($notes.Count -eq 0) { exit 0 }

$context = "Environnement MixFlow — points a corriger avant de lancer des commandes :`n- " + ($notes -join "`n- ")

# JSON strict sur stdout : seul l'exit 0 est parse par Claude Code.
$payload = @{
    hookSpecificOutput = @{
        hookEventName     = 'SessionStart'
        additionalContext = $context
    }
}

[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::Out.Write(($payload | ConvertTo-Json -Depth 5 -Compress))

exit 0
