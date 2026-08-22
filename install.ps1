$ErrorActionPreference = "Stop"

# Corros installer for Windows
# Run with: irm https://raw.githubusercontent.com/CocoCopi/corros/main/install.ps1 | iex

# Detect Architecture
$arch = $env:PROCESSOR_ARCHITECTURE.ToLower()
if ($arch -eq "amd64") {
    $arch = "x86_64"
} elseif ($arch -eq "arm64") {
    $arch = "aarch64"
} else {
    Write-Error "corros: unsupported architecture '$arch'"
    exit 1
}

$os = "windows"

# Set up installation prefix
$prefix = $env:PREFIX
if ([string]::IsNullOrWhiteSpace($prefix)) {
    $prefix = Join-Path $env:USERPROFILE ".local"
}
$bindir = Join-Path $prefix "bin"
if (-not (Test-Path $bindir)) {
    New-Item -ItemType Directory -Force -Path $bindir | Out-Null
}

$tmpDir = Join-Path $env:TEMP "corros_install_$(Get-Random)"
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

try {
    $asset = "corros-${os}-${arch}"
    $url = "https://github.com/CocoCopi/corros/releases/latest/download/${asset}.tar.gz"
    $tarPath = Join-Path $tmpDir "pkg.tar.gz"

    Write-Host "Downloading prebuilt corros (${os}/${arch}) from GitHub Releases..."
    try {
        Invoke-WebRequest -Uri $url -OutFile $tarPath -UseBasicParsing -ErrorAction Stop
        $downloaded = $true
    } catch {
        $downloaded = $false
        Write-Host "No prebuilt binary found for ${os}/${arch}."
    }

    if ($downloaded) {
        Write-Host "Extracting archive..."
        # Windows 10 1803 and later have tar.exe built-in
        & tar -xzf $tarPath -C $tmpDir
        
        $bin = Join-Path $tmpDir "corros.exe"
        if (-not (Test-Path $bin)) {
            $bin = Join-Path $tmpDir "corros"
        }
        $srcDir = $tmpDir
    } else {
        Write-Host "Building from source (needs Rust)..."
        if (-not (Get-Command cargo -ErrorAction Ignore)) {
            Write-Error "corros: 'cargo' not found. Install Rust first: https://rustup.rs/"
            exit 1
        }
        $repo = Join-Path $tmpDir "corros"
        
        $gitCloneProc = Start-Process git -ArgumentList "clone --depth 1 https://github.com/CocoCopi/corros.git $repo" -Wait -PassThru -NoNewWindow
        
        if ($gitCloneProc.ExitCode -ne 0) {
            # Try to build from current directory if we are inside corros repo
            if ((Test-Path "Cargo.toml") -and (Test-Path "src/prelude.cro")) {
                $repo = (Get-Location).Path
            } else {
                Write-Error "corros: could not obtain the sources"
                exit 1
            }
        }
        
        Push-Location $repo
        try {
            $cargoProc = Start-Process cargo -ArgumentList "build --release" -Wait -PassThru -NoNewWindow
            if ($cargoProc.ExitCode -ne 0) {
                Write-Error "corros: build failed"
                exit 1
            }
        } finally {
            Pop-Location
        }
        
        $bin = Join-Path $repo "target\release\corros.exe"
        $srcDir = Join-Path $repo "src"
    }

    # Install
    Write-Host "Installing to $bindir ..."
    Copy-Item -Path $bin -Destination (Join-Path $bindir "corros.exe") -Force
    
    $croFiles = @("compiler.cro", "vm.cro", "cli.cro", "prelude.cro", "codegen.cro")
    foreach ($cro in $croFiles) {
        $croPath = Join-Path $srcDir $cro
        if (Test-Path $croPath) {
            Copy-Item -Path $croPath -Destination (Join-Path $bindir $cro) -Force
        } else {
            Write-Warning "corros: warning - $cro not found; some features will fail"
        }
    }

    Write-Host "`nInstalled: $(Join-Path $bindir 'corros.exe')"
    
    # Check if bindir is in PATH
    $userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($null -eq $userPath) { $userPath = "" }
    $machinePath = [Environment]::GetEnvironmentVariable("PATH", "Machine")
    if ($null -eq $machinePath) { $machinePath = "" }
    
    $inPath = $false
    if (($userPath -split ';') -contains $bindir -or ($machinePath -split ';') -contains $bindir) {
        $inPath = $true
    }
    
    if (-not $inPath) {
        Write-Host "Adding $bindir to your User PATH..."
        $newUserPath = $userPath
        if ($newUserPath -and -not $newUserPath.EndsWith(";")) {
            $newUserPath += ";"
        }
        $newUserPath += $bindir
        [Environment]::SetEnvironmentVariable("PATH", $newUserPath, "User")
        $env:PATH = "$bindir;$env:PATH"
        Write-Host "Path updated. You may need to restart your terminal to use 'corros'."
    } else {
        Write-Host "corros is already in your PATH."
    }

    Write-Host "`nTry it: echo 'speak(`"corros works!`")' | corros"
    & (Join-Path $bindir "corros.exe") -v
} finally {
    if (Test-Path $tmpDir) {
        Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction Ignore
    }
}
