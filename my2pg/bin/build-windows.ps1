# Run from a Windows Developer PowerShell with Rust and Python installed.
$ErrorActionPreference = 'Stop'
if (Get-Command py -ErrorAction SilentlyContinue) {
    & py -3 "$PSScriptRoot/build-release.py" --platform windows @args
} else {
    & python "$PSScriptRoot/build-release.py" --platform windows @args
}
exit $LASTEXITCODE
