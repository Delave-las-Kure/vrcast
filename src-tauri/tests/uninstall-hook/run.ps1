# T356 — run the uninstaller hook's four cases and check what each one did.
#
# The macro names a real path, because the uninstaller cannot ask the application anything: it
# runs when the application is already going away. So this puts the real directory aside
# first, stands a marker in its place, runs a case, looks at the marker, and puts everything
# back — including when a case fails.
#
# **The case that earns this whole file** is the third: an installer of a new version runs the
# old uninstaller with `/UPDATE`, so the hook fires during an ordinary update. Without the
# guard it would take every server profile with it, and nobody would connect the loss to the
# update.

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$makensis = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\Bin\makensis.exe'
$real = Join-Path $env:APPDATA 'VRCast\VRCast Studio'
$aside = Join-Path $env:TEMP ("vrcast-hook-aside-" + [guid]::NewGuid().ToString('N'))

# Not run rather than failed when the compiler is absent: it arrives with the first full
# bundle build, and a machine that has never made one has nothing to check here. The same
# shape as the reference check — said out loud, and never passed off as a pass.
if (-not (Test-Path $makensis)) {
    Write-Host "--- the uninstall hook: NOT CHECKED (makensis is not there: $makensis) ---"
    Write-Host "    It arrives with the first `npx tauri build`."
    exit 0
}

# What each case must leave behind. The marker stands for everything the directory holds.
$cases = @(
    @{ checkbox = 1; update = 0; survives = $false; why = 'ticked, an ordinary removal: the data goes' }
    @{ checkbox = 0; update = 0; survives = $true;  why = 'not ticked: the data stays' }
    @{ checkbox = 1; update = 1; survives = $true;  why = 'ticked, but an update: the data stays' }
    @{ checkbox = 0; update = 1; survives = $true;  why = 'neither: the data stays' }
)

$moved = $false
try {
    if (Test-Path $real) {
        Move-Item $real $aside
        $moved = $true
        Write-Host "the real directory is aside: $aside"
    }

    $failed = 0
    foreach ($c in $cases) {
        New-Item -ItemType Directory -Force -Path $real | Out-Null
        $marker = Join-Path $real 'marker.txt'
        'данные' | Set-Content -LiteralPath $marker -Encoding UTF8

        $exe = Join-Path $env:TEMP ("vrcast-hook-" + $c.checkbox + $c.update + ".exe")
        $env:HOOK_TEST_OUT = $exe
        $env:HOOK_TEST_CHECKBOX = $c.checkbox
        $env:HOOK_TEST_UPDATE = $c.update

        & $makensis '/V1' (Join-Path $here 'hook-truth-table.nsi') | Out-Null
        if ($LASTEXITCODE -ne 0) { Write-Error "the hook would not compile for case $($c.why)" }

        & $exe | Out-Null
        Start-Sleep -Milliseconds 200

        $survived = Test-Path $marker
        if ($survived -eq $c.survives) {
            Write-Host ("  ok    " + $c.why)
        } else {
            Write-Host ("  WRONG " + $c.why + " — the marker " + $(if ($survived) { 'survived' } else { 'is gone' }))
            $failed++
        }
        Remove-Item -Force $exe -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force $real -ErrorAction SilentlyContinue
    }
}
finally {
    # Put it back whatever happened. A failing check that also loses the data would be worse
    # than the fault it was looking for.
    Remove-Item -Recurse -Force $real -ErrorAction SilentlyContinue
    if ($moved) {
        Move-Item $aside $real
        Write-Host "the real directory is back: $real"
    }
}

if ($failed -gt 0) {
    Write-Error "$failed of $($cases.Count) cases behaved wrongly"
}
Write-Host "--- the uninstall hook: all four cases behave ---"
