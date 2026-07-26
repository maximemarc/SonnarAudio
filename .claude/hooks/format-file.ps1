# PostToolUse (Edit|Write) — formate le fichier touché avec l'outil du
# projet, pour que lint-staged / la CI n'aient rien à redire.
# Ce hook ne bloque JAMAIS : il sort toujours en 0.

. "$PSScriptRoot\_common.ps1"

$payload = Read-HookInput
if ($null -eq $payload) { exit 0 }

$file = $payload.tool_input.file_path
if ([string]::IsNullOrWhiteSpace($file) -or -not (Test-Path $file)) { exit 0 }

$root = Get-ProjectDir

# Ne rien faire hors du dépôt, ni dans les artefacts de build.
if ($file -notlike "$root*") { exit 0 }
if ($file -match '(?i)[\\/](node_modules|dist|target|gen)[\\/]') { exit 0 }

Initialize-MixFlowPath
$ext = [System.IO.Path]::GetExtension($file).ToLowerInvariant()

try {
    switch ($ext) {
        '.rs' {
            # rustfmt lit src-tauri/rustfmt.toml via --config-path.
            $cfg = Join-Path $root 'src-tauri'
            & rustfmt --edition 2021 --config-path $cfg $file 2>$null | Out-Null
        }
        { $_ -in '.ts', '.tsx', '.json', '.css', '.md', '.html' } {
            Push-Location $root
            try {
                & npx --no-install prettier --write --log-level warn $file 2>$null | Out-Null
            } finally {
                Pop-Location
            }
        }
        default { }
    }
} catch {
    # Outil absent ou fichier temporairement invalide : on laisse passer,
    # `npm run format:check` / `cargo fmt --check` restent le filet.
}

exit 0
