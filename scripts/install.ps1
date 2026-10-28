# Installs Card English for the current user from the latest GitHub release, or updates it: no administrator
# rights, a shortcut in the Start menu, and an entry in Settings, Apps that removes it.
#
#   irm https://raw.githubusercontent.com/eliasbui/app-card-english/main/scripts/install.ps1 | iex
#
# The words, settings and models in %APPDATA%\CardEnglish stay when the app is updated or removed. The script runs
# in a block of its own, so that iex leaves no variables or preferences behind in the session.
& {
    $ErrorActionPreference = 'Stop'
    # Invoke-WebRequest is many times slower while it draws its progress bar.
    $ProgressPreference = 'SilentlyContinue'
    # Windows PowerShell 5.1 may not offer TLS 1.2 by default, which GitHub requires.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'eliasbui/app-card-english'
    $dir = Join-Path $env:LOCALAPPDATA 'Programs\CardEnglish'
    $exe = Join-Path $dir 'CardEnglish.exe'
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Card English.lnk'
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\CardEnglish'
    function Quote($text) { "'" + $text.Replace("'", "''") + "'" }

    $headers = @{}
    # CI machines share their address, which the API limits unless the request carries a token.
    if ($env:GITHUB_TOKEN) { $headers.Authorization = "Bearer $env:GITHUB_TOKEN" }
    # The newest release that has the Windows zip.
    $releases = Invoke-RestMethod "https://api.github.com/repos/$repo/releases?per_page=10" -Headers $headers
    $release = $releases | Where-Object { -not $_.draft -and -not $_.prerelease -and ($_.assets | Where-Object name -Like 'CardEnglish-*-windows-x64.zip') } | Select-Object -First 1
    if (-not $release) { throw 'Chưa có bản nào cho Windows.' }
    $asset = $release.assets | Where-Object name -Like 'CardEnglish-*-windows-x64.zip' | Select-Object -First 1
    $version = $release.tag_name.TrimStart('v')
    Write-Host "Đang tải Card English $version..."

    $zip = Join-Path ([IO.Path]::GetTempPath()) $asset.name
    Invoke-WebRequest $asset.browser_download_url -OutFile $zip -UseBasicParsing
    # GitHub publishes the SHA-256 of every file of a release; -ne ignores the case of the hex digits.
    if ($asset.digest -and $asset.digest -ne "sha256:$((Get-FileHash $zip -Algorithm SHA256).Hash)") {
        Remove-Item $zip
        throw 'File tải về bị hỏng, hãy chạy lại lệnh.'
    }

    # A running copy holds its file: it is closed now and opened again once it is replaced.
    $running = @(Get-Process CardEnglish -ErrorAction SilentlyContinue | Where-Object Path -EQ $exe)
    $running | Stop-Process -Force -PassThru | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $dir | Out-Null
    Expand-Archive $zip -DestinationPath $dir -Force
    Remove-Item $zip

    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    $link.TargetPath = $exe
    $link.WorkingDirectory = $dir
    $link.Save()

    # What Settings, Apps runs to remove it: the app, its shortcut and its login item, never the data.
    $remove = "Get-Process CardEnglish -ErrorAction SilentlyContinue | Where-Object Path -EQ $(Quote $exe) | " +
        'Stop-Process -Force -PassThru | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue; ' +
        "Remove-Item -Recurse -Force $(Quote $dir), $(Quote $shortcut) -ErrorAction SilentlyContinue; " +
        "Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' 'Card English' -ErrorAction SilentlyContinue; " +
        "Remove-Item $(Quote $key)"
    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $entry = @{
        DisplayName = 'Card English'
        DisplayVersion = $version
        Publisher = 'eliasbui'
        DisplayIcon = $exe
        InstallLocation = $dir
        # Encoded, so that no path needs quoting on the command line.
        UninstallString = "`"$powershell`" -NoProfile -WindowStyle Hidden -EncodedCommand " +
            [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($remove))
    }
    New-Item $key -Force | Out-Null
    foreach ($name in $entry.Keys) { Set-ItemProperty $key $name $entry[$name] }

    if ($running) { Start-Process $exe }
    Write-Host "Đã cài Card English $version vào $dir."
    Write-Host 'Mở app từ menu Start. Chạy lại lệnh này để cập nhật; gỡ app trong Settings > Apps.'
}
