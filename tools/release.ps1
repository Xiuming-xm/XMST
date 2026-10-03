#Requires -Version 5.1
<#
.SYNOPSIS
    XMST 本地发版脚本：构建 → 备份 → 交付 → 校验 → 提交 →（可选）打标签 / 推送。

.DESCRIPTION
    一条命令完成发版，并把「容易忘的校验」固化下来（依据 AGENTS.md §3 / §7）：

      * TMP/TEMP 指向工作区内的 temp\ctmp（否则部分 build script 会因权限失败）
      * dist\XMST-<版本>.exe 与 target\release\xmst.exe 的 SHA256 必须一致
      * 交付物的完整性标签必须 Medium（Low 会让程序被降级运行，见 AGENTS.md §4.1）
      * 检查 Zone.Identifier（MOTW，即「无法验证发布者」提示的来源）并自动解除
      * 旧产物自动备份到 dist\backup\<yyyyMMdd_HHmmss>\，只保留最近 2 份备份目录
      * 产物同时落到 dist\ 与 versions\<版本>\（含 SHA256.txt），versions\ 会一起提交

    默认**不推送**：只有显式加 -Push 才会 git push。

    本脚本只用 PowerShell 内置 cmdlet 与 git / cargo / icacls；
    不调用 Set-ExecutionPolicy、不使用 Add-Type、不下载任何东西。

.PARAMETER Version
    版本号，例如 0.1.2-alpha。不传则读取 Cargo.toml 中 [package] 的 version。

.PARAMETER SkipBuild
    跳过 cargo check 与 cargo build --release，直接使用已有的 target\release\xmst.exe。

.PARAMETER Push
    执行 git push（推 master，并在 -Tag 时推送标签）。默认不推送。

.PARAMETER Tag
    创建并（配合 -Push）推送标签 v<版本>。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File F:\XMST\tools\release.ps1
    只构建 + 部署 + 提交，不推送（版本号取 Cargo.toml）。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File F:\XMST\tools\release.ps1 -Version 0.1.2-alpha -Push -Tag
    指定版本、提交并推送 master 与标签 v0.1.2-alpha（网络打不开 GitHub 网页时最常用）。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File F:\XMST\tools\release.ps1 -SkipBuild
    复用已构建的 exe 重新打包交付（跳过 cargo，会提示产物是否比源码旧）。

.NOTES
    使用说明与常见问题：docs\发版流程.md
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Version,

    [switch]$SkipBuild,
    [switch]$Push,
    [switch]$Tag
)

$ErrorActionPreference = 'Stop'

# 兜底：任何未预期的终止性错误都以清晰的中文报错 + 退出码 1 结束（绝不静默继续）
$script:Aborted = $false
trap {
    Write-Host ''
    if (-not $script:Aborted) {
        Write-Host ('  [失败] 未处理的错误：' + $_.Exception.Message) -ForegroundColor Red
        if ($_.InvocationInfo -and $_.InvocationInfo.PositionMessage) {
            Write-Host ('         ' + ($_.InvocationInfo.PositionMessage -replace '\s+', ' ').Trim()) -ForegroundColor DarkGray
        }
    }
    Write-Host ''
    Write-Host '发版已中止（后续步骤未执行）。' -ForegroundColor Red
    exit 1
}

# 尽量让中文与 git/cargo 的 UTF-8 输出正常显示（失败不影响流程）
try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false) } catch { }
try { $OutputEncoding = New-Object System.Text.UTF8Encoding($false) } catch { }

# ---------------------------------------------------------------- 常量 / 路径

$TotalSteps   = 10
$CommitPhrase = '交付产物与版本号同步'          # 提交信息 = "release: <版本> <此短语>"
$Utf8NoBom    = New-Object System.Text.UTF8Encoding($false)
$Utf8Bom      = New-Object System.Text.UTF8Encoding($true)

if (-not $PSScriptRoot) {
    Write-Host '[失败] 无法确定脚本所在目录（$PSScriptRoot 为空），请用 -File 方式运行本脚本。' -ForegroundColor Red
    exit 1
}

# 项目根 = 本脚本所在目录的上一级（不依赖调用者的当前目录，可在任意目录调用本脚本）
# 注意：PowerShell 5.1 的 Split-Path 把 -LiteralPath 放在**独立参数集**里，不能与 -Parent/-Leaf
# 同用（会报 "Parameter set cannot be resolved"），所以路径拆分统一改用 .NET；
# 顺带也不会把路径里的 [ ] 当通配符（项目里踩过这个坑）。
$ProjectRoot = [System.IO.Path]::GetDirectoryName($PSScriptRoot)
$CargoToml   = Join-Path $ProjectRoot 'Cargo.toml'
$TargetExe   = Join-Path $ProjectRoot 'target\release\xmst.exe'
$SrcDir      = Join-Path $ProjectRoot 'src'
$DistDir     = Join-Path $ProjectRoot 'dist'
$BackupRoot  = Join-Path $DistDir 'backup'
$VersionsDir = Join-Path $ProjectRoot 'versions'
$TempDir     = Join-Path $ProjectRoot 'temp\ctmp'

$StartTime       = Get-Date
$script:LastExit = 0

# ---------------------------------------------------------------- 输出助手

function Write-Section {
    param([Parameter(Mandatory = $true)][string]$Text)
    Write-Host ''
    Write-Host ('==== ' + $Text + ' ====') -ForegroundColor Cyan
}

function Write-Step {
    param(
        [Parameter(Mandatory = $true)][int]$Number,
        [Parameter(Mandatory = $true)][string]$Text
    )
    Write-Host ''
    Write-Host ('[' + $Number + '/' + $TotalSteps + '] ' + $Text) -ForegroundColor Cyan
}

function Write-Ok   { param([string]$Text) Write-Host ('  [OK]   ' + $Text) -ForegroundColor Green }
function Write-Info { param([string]$Text) Write-Host ('  ·      ' + $Text) }
function Write-Warn { param([string]$Text) Write-Host ('  [警告] ' + $Text) -ForegroundColor Yellow }
function Write-Todo { param([string]$Text) Write-Host ('  [待办] ' + $Text) -ForegroundColor Magenta }

function Fail {
    param(
        [Parameter(Mandatory = $true)][string]$Message,
        [string[]]$Hints = @()
    )
    $script:Aborted = $true
    Write-Host ''
    Write-Host ('  [失败] ' + $Message) -ForegroundColor Red
    foreach ($h in $Hints) { Write-Host ('         ' + $h) -ForegroundColor Yellow }
    Write-Host ''
    Write-Host '发版已中止（后续步骤未执行，已完成的步骤不受影响）。' -ForegroundColor Red
    exit 1
}

# ---------------------------------------------------------------- 外部命令助手

# 执行外部命令并**直接透传**其 stdout/stderr 到控制台（用于 cargo 这类需要实时看输出的命令），
# 同时校验退出码；失败即中止（除非显式 -AllowFailure）。
function Invoke-NativeStream {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @(),
        [Parameter(Mandatory = $true)][string]$What,
        [switch]$AllowFailure
    )
    Write-Host ('  > ' + $FilePath + ' ' + ($Arguments -join ' ')) -ForegroundColor DarkGray
    & $FilePath @Arguments
    $script:LastExit = $LASTEXITCODE
    if ($script:LastExit -ne 0) {
        if ($AllowFailure) {
            Write-Warn ($What + ' 退出码 ' + $script:LastExit + '（按容错处理）')
            return
        }
        Fail ($What + ' 失败（退出码 ' + $script:LastExit + '）')
    }
}

# 执行外部命令并捕获输出（用于 git ls-remote / icacls 这类需要解析文本的命令）。
# 注意：捕获时临时把 ErrorActionPreference 设为 Continue，避免 PowerShell 5.1 把
# native stderr 合并输出（2>&1）当成终止性错误。
function Invoke-NativeCapture {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @()
    )
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $raw  = & $FilePath @Arguments 2>&1
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    $lines = @()
    if ($null -ne $raw) {
        foreach ($item in @($raw)) { $lines += [string]$item }
    }
    $script:LastExit = $code
    return [pscustomobject]@{
        ExitCode = $code
        Lines    = $lines
        Text     = ($lines -join "`r`n")
    }
}

# ---------------------------------------------------------------- 领域助手

# 读取 Cargo.toml 中 [package] 段的 version（只认行首的 version = "..."，不会误取依赖里的 version）
function Get-CargoVersion {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { Fail ('找不到 Cargo.toml：' + $Path) }
    $lines  = [System.IO.File]::ReadAllLines($Path, [System.Text.Encoding]::UTF8)
    $inPkg  = $false
    foreach ($line in $lines) {
        $t = $line.Trim()
        if ($t -eq '[package]') { $inPkg = $true; continue }
        if ($inPkg -and $t.StartsWith('[')) { break }
        if ($inPkg -and ($t -match '^version\s*=\s*"([^"]+)"')) { return $Matches[1] }
    }
    return $null
}

# 把 Cargo.toml 中 [package] 的 version 改写为新值（保留原有换行风格与末尾换行，写回时不加 BOM）
function Set-CargoVersion {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$NewVersion
    )
    $raw   = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
    $lines = [System.IO.File]::ReadAllLines($Path, [System.Text.Encoding]::UTF8)

    $eol = "`n"
    if ($raw.Contains("`r`n")) { $eol = "`r`n" }
    $trailing = $raw.EndsWith("`n")

    $inPkg = $false
    $index = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $t = $lines[$i].Trim()
        if ($t -eq '[package]') { $inPkg = $true; continue }
        if ($inPkg -and $t.StartsWith('[')) { break }
        if ($inPkg -and ($t -match '^version\s*=')) { $index = $i; break }
    }
    if ($index -lt 0) { Fail ('无法在 Cargo.toml 的 [package] 段中找到 version 行：' + $Path) }

    $lines[$index] = 'version = "' + $NewVersion + '"'
    $text = ($lines -join $eol)
    if ($trailing) { $text += $eol }

    [System.IO.File]::WriteAllText($Path, $text, $Utf8NoBom)
}

# 查询完整性标签：先看文件自身，没有独立标签则逐级向上看目录，返回第一个找到的标签
function Get-IntegrityLabelInfo {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)
    $result = [pscustomobject]@{
        Found      = $false
        Level      = ''
        SourcePath = ''
        RawLine    = ''
    }
    $candidate = $LiteralPath
    $guard     = 0
    while ($candidate -and $guard -lt 64) {
        $guard++
        $cap = Invoke-NativeCapture -FilePath 'icacls' -Arguments @($candidate)
        if ($cap.ExitCode -eq 0) {
            foreach ($line in $cap.Lines) {
                if (($line -match 'Mandatory Label') -or ($line -match 'Mandatory Level') -or
                    ($line -match '强制标签') -or ($line -match '强制级别')) {
                    $level = 'Unknown'
                    if (($line -match '(?i)\bLow\b') -or ($line -match '低'))         { $level = 'Low' }
                    elseif (($line -match '(?i)\bMedium\b') -or ($line -match '中'))  { $level = 'Medium' }
                    elseif (($line -match '(?i)\bHigh\b') -or ($line -match '高'))    { $level = 'High' }
                    elseif (($line -match '(?i)\bSystem\b') -or ($line -match '系统')) { $level = 'System' }
                    $result.Found      = $true
                    $result.Level      = $level
                    $result.SourcePath = $candidate
                    $result.RawLine    = $line.Trim()
                    return $result
                }
            }
        }
        $parent = [System.IO.Path]::GetDirectoryName($candidate)
        if ((-not $parent) -or ($parent -eq $candidate)) { break }
        $candidate = $parent
    }
    return $result
}

# 是否带 Zone.Identifier（MOTW）
function Test-Motw {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)
    try {
        $item = Get-Item -LiteralPath $LiteralPath -Stream 'Zone.Identifier' -ErrorAction Stop
        foreach ($s in @($item)) {
            if ($s.Stream -eq 'Zone.Identifier') { return $true }
        }
        return $false
    } catch {
        return $false
    }
}

# 创建目录：New-Item 在 PowerShell 5.1 上**没有** -LiteralPath 参数（用了会直接报错），
# 所以这里用 .NET API：5.1/7.x 通用，且路径里的 [ ] 不会被当成通配符
function Ensure-Directory {
    param([Parameter(Mandatory = $true)][string]$Path)
    $null = [System.IO.Directory]::CreateDirectory($Path)
}

# 复制文件，带重试（杀软/索引器短暂占用时更稳）
function Copy-FileWithRetry {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [int]$Attempts = 3
    )
    for ($i = 1; $i -le $Attempts; $i++) {
        try {
            Copy-Item -LiteralPath $Source -Destination $Destination -Force -ErrorAction Stop
            return
        } catch {
            if ($i -ge $Attempts) {
                Fail ('复制文件失败：' + $Source + ' → ' + $Destination) @(
                    $_.Exception.Message,
                    '请确认没有其它进程占用该文件（可重跑本脚本重试）。'
                )
            }
            Write-Warn ('复制失败（第 ' + $i + ' 次）：' + $_.Exception.Message + ' —— 500ms 后重试')
            Start-Sleep -Milliseconds 500
        }
    }
}

# ---------------------------------------------------------------- 开场

Write-Host ''
Write-Host 'XMST 本地发版脚本' -ForegroundColor White
Write-Host ('  项目根目录 : ' + $ProjectRoot)
Write-Host ('  开始时间   : ' + $StartTime.ToString('yyyy-MM-dd HH:mm:ss'))
Write-Host ('  参数       : Version=' + ($(if ($Version) { $Version } else { '(取 Cargo.toml)' })) +
            ' SkipBuild=' + [bool]$SkipBuild + ' Push=' + [bool]$Push + ' Tag=' + [bool]$Tag)

if (-not (Test-Path -LiteralPath $CargoToml)) {
    Fail ('在 ' + $ProjectRoot + ' 下找不到 Cargo.toml。') @(
        '本脚本必须放在 <项目根>\tools\ 目录下（用 $PSScriptRoot 的上一级推导项目根）。'
    )
}

# SSH 参数：BatchMode 避免交互式卡死；本脚本所有 git 远端操作都会带上它
$env:GIT_SSH_COMMAND = 'ssh -o BatchMode=yes -o ConnectTimeout=15'
Write-Host ('  GIT_SSH_COMMAND = ' + $env:GIT_SSH_COMMAND)

# ---------------------------------------------------------------- 1/10 工作区状态

Write-Step -Number 1 -Text '检查工作区状态（git status）'

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Fail '找不到 git（请确认 git 在 PATH 中）。'
}

$branchCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'rev-parse', '--abbrev-ref', 'HEAD')
if ($branchCap.ExitCode -ne 0) {
    Fail ('git rev-parse 失败：' + $branchCap.Text) @('请确认这是一个有效的 git 仓库。')
}
$Branch = ($branchCap.Text -replace '\s+', ' ').Trim()
Write-Info ('当前分支：' + $Branch)
if ($Branch -ne 'master') {
    Write-Warn "当前不在 master 分支上；带 -Push 时脚本会拒绝推送（发版只在 master 上做）。"
}

$statusCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'status', '--porcelain')
if ($statusCap.ExitCode -ne 0) {
    Fail ('git status 失败：' + $statusCap.Text)
}
$changed = @($statusCap.Lines | Where-Object { $_.Trim().Length -gt 0 })
if ($changed.Count -eq 0) {
    Write-Ok '工作区干净（没有未提交改动）'
} else {
    Write-Warn ('工作区有 ' + $changed.Count + ' 处未提交改动：脚本会继续，并在第 8 步把它们一起提交。')
    foreach ($line in $changed) { Write-Info ('    ' + $line) }
}

# ---------------------------------------------------------------- 2/10 版本号

Write-Step -Number 2 -Text '确定版本号 / 同步 Cargo.toml'

if ($null -ne $Version) { $Version = $Version.Trim() }

$cargoVersion = Get-CargoVersion -Path $CargoToml
if (-not $cargoVersion) {
    Fail ('无法从 ' + $CargoToml + ' 的 [package] 段读取 version。')
}
Write-Info ('Cargo.toml 当前 version = ' + $cargoVersion)

if (-not $Version) {
    $Version = $cargoVersion
    Write-Ok ('未指定 -Version，使用 Cargo.toml 的版本：' + $Version)
} else {
    if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z\.\-\+]*$') {
        Fail ("版本号不合法：" + $Version) @(
            '只允许字母/数字/点/连字符/加号，且不能以符号开头（因为要用于文件名与目录名）。',
            '示例：0.1.2-alpha'
        )
    }
    if ($Version -eq $cargoVersion) {
        Write-Ok ('版本一致：' + $Version)
    } else {
        Write-Warn ('-Version（' + $Version + '）与 Cargo.toml（' + $cargoVersion + '）不一致。')
        Write-Info ('以 -Version 为准，更新 Cargo.toml ...')
        Set-CargoVersion -Path $CargoToml -NewVersion $Version
        $check = Get-CargoVersion -Path $CargoToml
        if ($check -ne $Version) {
            Fail ('更新 Cargo.toml 后校验失败：仍读到 ' + $check)
        }
        Write-Ok ('Cargo.toml 已改为 version = "' + $Version + '"')
    }
}

if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+') {
    Write-Warn ('版本号 "' + $Version + '" 看起来不像 x.y.z 形式，请确认这是有意为之。')
}

$DistExe    = Join-Path $DistDir ('XMST-' + $Version + '.exe')
$VersionDir = Join-Path $VersionsDir $Version
$VersionExe = Join-Path $VersionDir ('XMST-' + $Version + '.exe')
$ShaFile    = Join-Path $VersionDir 'SHA256.txt'
$TagName    = 'v' + $Version

# ---------------------------------------------------------------- 3/10 构建

Write-Step -Number 3 -Text '构建（TMP/TEMP 指向工作区内的 temp\ctmp）'

Ensure-Directory -Path $TempDir
$env:TMP  = $TempDir
$env:TEMP = $TempDir
Write-Info ('TMP/TEMP = ' + $TempDir)

if ($SkipBuild) {
    Write-Warn '已指定 -SkipBuild：跳过 cargo check 与 cargo build --release。'
    if (-not (Test-Path -LiteralPath $TargetExe)) {
        Fail ('找不到 ' + $TargetExe) @('-SkipBuild 需要已有构建产物；请去掉 -SkipBuild 重新发版。')
    }
    $exeTime = (Get-Item -LiteralPath $TargetExe).LastWriteTime
    Write-Info ('现有产物时间：' + $exeTime.ToString('yyyy-MM-dd HH:mm:ss'))

    $newest = @(Get-ChildItem -LiteralPath $SrcDir -Recurse -File -Filter '*.rs' -ErrorAction SilentlyContinue |
                Sort-Object -Property LastWriteTime -Descending)
    $cargoTime = (Get-Item -LiteralPath $CargoToml).LastWriteTime
    if ($cargoTime -gt $exeTime) {
        Write-Warn 'Cargo.toml 比产物新（若只是刚改了版本号则无所谓；若改了依赖/配置，产物可能已过期）。'
    }
    if ($newest.Count -gt 0 -and $newest[0].LastWriteTime -gt $exeTime) {
        Write-Warn ('源码 ' + $newest[0].Name + '（' + $newest[0].LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss') +
                    '）比产物新：产物可能未包含最新改动，建议去掉 -SkipBuild。')
    }
} else {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Fail '找不到 cargo（请确认 Rust 工具链在 PATH 中）。'
    }
    Push-Location -LiteralPath $ProjectRoot
    try {
        Write-Info 'cargo check --message-format short'
        Invoke-NativeStream -FilePath 'cargo' -Arguments @('check', '--message-format', 'short') -What 'cargo check'
        Write-Ok 'cargo check 通过'

        Write-Info 'cargo build --release（约 3 分钟）'
        Invoke-NativeStream -FilePath 'cargo' -Arguments @('build', '--release') -What 'cargo build --release'
        Write-Ok 'cargo build --release 完成'
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $TargetExe)) {
    Fail ('构建后仍找不到 ' + $TargetExe)
}
Write-Ok ('构建产物：' + $TargetExe + '（' + (Get-Item -LiteralPath $TargetExe).Length + ' 字节）')

# ---------------------------------------------------------------- 4/10 结束进程

Write-Step -Number 4 -Text '结束正在运行的 XMST 进程'

$killed = 0
try {
    $procs = @(Get-Process -Name 'XMST*' -ErrorAction SilentlyContinue)
    if ($procs.Count -gt 0) {
        foreach ($p in $procs) {
            Write-Info ('结束进程 ' + $p.ProcessName + ' (PID ' + $p.Id + ')')
            try {
                Stop-Process -Id $p.Id -Force -ErrorAction Stop
                $killed++
            } catch {
                Write-Warn ('结束进程 ' + $p.Id + ' 失败：' + $_.Exception.Message)
            }
        }
        Start-Sleep -Milliseconds 500
    }
} catch {
    Write-Warn ('枚举 XMST 进程时出错（已忽略）：' + $_.Exception.Message)
}

$remain = @(Get-Process -Name 'XMST*' -ErrorAction SilentlyContinue)
if ($remain.Count -gt 0) {
    Fail ('仍有 ' + $remain.Count + ' 个 XMST 进程在运行，产物可能被占用。') @(
        '请手动关闭这些进程后重新运行本脚本。'
    )
}
if ($killed -gt 0) { Write-Ok ('已结束 ' + $killed + ' 个进程') } else { Write-Ok '没有正在运行的 XMST 进程' }

# ---------------------------------------------------------------- 5/10 备份旧产物

Write-Step -Number 5 -Text '备份旧产物（dist\backup\<时间戳>\，保留最近 2 份）'

Ensure-Directory -Path $DistDir
Ensure-Directory -Path $BackupRoot

$BackupDir = ''
$oldExes = @(Get-ChildItem -LiteralPath $DistDir -Filter 'XMST-*.exe' -File -ErrorAction SilentlyContinue)
if ($oldExes.Count -gt 0) {
    $stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
    $BackupDir = Join-Path $BackupRoot $stamp
    Ensure-Directory -Path $BackupDir
    foreach ($f in $oldExes) {
        Copy-FileWithRetry -Source $f.FullName -Destination (Join-Path $BackupDir $f.Name)
        Write-Info ('已备份 ' + $f.Name)
    }
    Write-Ok ('备份目录：' + $BackupDir)
} else {
    Write-Info 'dist 下没有旧的 XMST-*.exe，跳过备份。'
}

# 只保留最近 2 份「时间戳」备份目录（dist\backup 里的散装历史文件不动）
$backupDirs = @(Get-ChildItem -LiteralPath $BackupRoot -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -match '^\d{8}_\d{6}$' } |
                Sort-Object -Property Name -Descending)
if ($backupDirs.Count -gt 2) {
    $backupDirs | Select-Object -Skip 2 | ForEach-Object {
        try {
            Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction Stop
            Write-Info ('已清理旧备份：' + $_.Name)
        } catch {
            Write-Warn ('清理旧备份 ' + $_.Name + ' 失败（已忽略）：' + $_.Exception.Message)
        }
    }
}
Write-Ok ('当前保留备份目录 ' + [Math]::Min($backupDirs.Count, 2) + ' 份')

# 提示：dist 里残留的旧版本 exe 不会被自动删除（保守起见），但会让交付目录变乱
$stale = @($oldExes | Where-Object { $_.Name -ne ('XMST-' + $Version + '.exe') })
if ($stale.Count -gt 0) {
    Write-Info ('注意：dist 下仍有旧版本产物（已备份，未删除）：' + (($stale | ForEach-Object { $_.Name }) -join '、'))
}

# ---------------------------------------------------------------- 6/10 生成产物

Write-Step -Number 6 -Text ('生成交付产物 XMST-' + $Version + '.exe（dist + versions）')

Ensure-Directory -Path $VersionDir

Copy-FileWithRetry -Source $TargetExe -Destination $DistExe
Write-Ok ('dist      → ' + $DistExe)

Copy-FileWithRetry -Source $TargetExe -Destination $VersionExe
Write-Ok ('versions  → ' + $VersionExe)

# SHA256.txt：与 versions\0.1.0-alpha\SHA256.txt 保持一致的格式（UTF-8 带 BOM + CRLF）
$hashUpper = (Get-FileHash -LiteralPath $VersionExe -Algorithm SHA256).Hash.ToUpperInvariant()
$shaText   = $hashUpper + '  XMST-' + $Version + '.exe' + "`r`n"
[System.IO.File]::WriteAllText($ShaFile, $shaText, $Utf8Bom)
Write-Ok ('SHA256.txt → ' + $ShaFile)

# ---------------------------------------------------------------- 7/10 交付前校验

Write-Step -Number 7 -Text '交付前校验（SHA256 / 完整性标签 / MOTW）'

# 7.1 SHA256 一致性
$hashTarget = (Get-FileHash -LiteralPath $TargetExe -Algorithm SHA256).Hash.ToUpperInvariant()
$hashDist   = (Get-FileHash -LiteralPath $DistExe   -Algorithm SHA256).Hash.ToUpperInvariant()
$hashVer    = (Get-FileHash -LiteralPath $VersionExe -Algorithm SHA256).Hash.ToUpperInvariant()

Write-Info ('target\release\xmst.exe : ' + $hashTarget)
Write-Info ('dist\XMST-' + $Version + '.exe : ' + $hashDist)
Write-Info ('versions\...\XMST-' + $Version + '.exe : ' + $hashVer)

if ($hashDist -ne $hashTarget) {
    Fail 'SHA256 不一致：dist 产物与 target\release\xmst.exe 不同！' @(
        '可能原因：复制被中断、杀软改写了文件、或构建产物被并发覆盖。',
        '请删除 dist 下的该 exe 后重新运行本脚本。'
    )
}
if ($hashVer -ne $hashTarget) {
    Fail 'SHA256 不一致：versions 产物与 target\release\xmst.exe 不同！'
}
if ((Get-Content -LiteralPath $ShaFile -Raw).Trim() -ne ($hashUpper + '  XMST-' + $Version + '.exe')) {
    Fail 'SHA256.txt 内容与产物哈希不一致。'
}
Write-Ok '三处 SHA256 完全一致，SHA256.txt 内容校验通过'

# 7.2 PE 头（避免复制出一个空壳/文本文件）
$fs = [System.IO.File]::OpenRead($DistExe)
try {
    $head = New-Object byte[] 2
    $null = $fs.Read($head, 0, 2)
} finally {
    $fs.Dispose()
}
if (($head[0] -ne 0x4D) -or ($head[1] -ne 0x5A)) {
    Fail ('dist 产物不是有效的 Windows 可执行文件（缺少 MZ 头）：' + $DistExe)
}
Write-Ok '产物 PE 头（MZ）正常'

# 7.3 完整性标签：必须 Medium（Low 会让程序被降级运行，见 AGENTS.md §4.1）
$ilDist = Get-IntegrityLabelInfo -LiteralPath $DistExe
if ($ilDist.Found) {
    Write-Info ('dist 完整性标签：' + $ilDist.Level + '（来源：' + $ilDist.SourcePath + '）')
    Write-Info ('    ' + $ilDist.RawLine)
} else {
    Write-Info 'dist 产物与其上级目录都没有显式完整性标签 → 按系统默认 Medium 处理'
}

if ($ilDist.Level -eq 'Low') {
    Fail ('交付物处于 Low 完整性级别（来源：' + $ilDist.SourcePath + '）—— 程序会被降级运行，文件与 shell 操作全部失效！') @(
        '修复命令（改完必须重启程序）：',
        ('  icacls "' + $ProjectRoot + '" /setintegritylevel M /T /C'),
        '详见 AGENTS.md §4.1 与 docs\发版流程.md'
    )
}
if ($ilDist.Level -eq 'High' -or $ilDist.Level -eq 'System' -or $ilDist.Level -eq 'Unknown') {
    Write-Warn ('完整性级别是 ' + $ilDist.Level + '（既不是 Medium 也不是 Low）：请人工确认是否合适。')
} elseif ($ilDist.Level -eq 'Medium') {
    Write-Ok '完整性标签检查通过（Medium）'
} else {
    Write-Ok '完整性标签检查通过（无显式标签 = 系统默认 Medium）'
}

# versions 产物同样检查（它是给外部下载的那一份）
$ilVer = Get-IntegrityLabelInfo -LiteralPath $VersionExe
if ($ilVer.Found) {
    Write-Info ('versions 完整性标签：' + $ilVer.Level + '（来源：' + $ilVer.SourcePath + '）')
} else {
    Write-Info 'versions 产物与其上级目录无显式标签 → 按系统默认 Medium 处理'
}
if ($ilVer.Level -eq 'Low') {
    Fail ('versions 交付物处于 Low 完整性级别（来源：' + $ilVer.SourcePath + '）！') @(
        '修复命令（改完必须重启程序）：',
        ('  icacls "' + $ProjectRoot + '" /setintegritylevel M /T /C')
    )
}

# 7.4 MOTW（Zone.Identifier）
foreach ($f in @($DistExe, $VersionExe)) {
    if (Test-Motw -LiteralPath $f) {
        Write-Warn ('检测到 Zone.Identifier（MOTW）：' + $f)
        try {
            Unblock-File -LiteralPath $f -ErrorAction Stop
            Write-Ok '  已解除（Unblock-File）'
        } catch {
            Fail ('解除 MOTW 失败：' + $f + ' —— ' + $_.Exception.Message)
        }
    } else {
        Write-Ok ('无 MOTW（Zone.Identifier）：' + [System.IO.Path]::GetFileName($f))
    }
}

# ---------------------------------------------------------------- 8/10 提交

Write-Step -Number 8 -Text ('git 提交（release: ' + $Version + '）')

Invoke-NativeStream -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'add', '-A') -What 'git add -A'

$stagedCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'status', '--porcelain')
$staged = @($stagedCap.Lines | Where-Object { $_.Trim().Length -gt 0 })

$Committed = $false
if ($staged.Count -eq 0) {
    Write-Warn '暂存区没有改动（无可提交内容）：跳过 git commit，沿用当前 HEAD。'
} else {
    $commitMessage = 'release: ' + $Version + ' ' + $CommitPhrase
    # 用 -F 传入 UTF-8 文件，避免命令行编码问题导致中文提交信息乱码
    $msgFile = Join-Path $TempDir 'release_commit_msg.txt'
    [System.IO.File]::WriteAllText($msgFile, $commitMessage + "`r`n", $Utf8NoBom)
    Invoke-NativeStream -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'commit', '-F', $msgFile) -What 'git commit'
    $Committed = $true
    Write-Ok ('已提交 ' + $staged.Count + ' 项改动，提交信息：' + $commitMessage)
}

$headCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'log', '-1', '--format=%h %s')
if ($headCap.ExitCode -ne 0) {
    Fail ('git log 失败：' + $headCap.Text)
}
$HeadLine = ($headCap.Text -replace '\s+', ' ').Trim()
Write-Ok ('当前 HEAD：' + $HeadLine)

$TagCreated = $false
if ($Tag) {
    $tagCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'tag', '-l', $TagName)
    if ($tagCap.Text.Trim().Length -gt 0) {
        Write-Warn ('标签 ' + $TagName + ' 已存在：不重复创建（如需移动请手动 git tag -f）。')
    } else {
        Invoke-NativeStream -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'tag', '-a', $TagName, '-m', ('release ' + $Version)) -What ('git tag ' + $TagName)
        $TagCreated = $true
        Write-Ok ('已创建标签：' + $TagName)
    }
}

# ---------------------------------------------------------------- 9/10 推送

Write-Step -Number 9 -Text '推送（只有显式加 -Push 才会执行）'

$Pushed = $false
if (-not $Push) {
    Write-Info '未指定 -Push：跳过推送（本地提交/标签已完成）。'
    Write-Info '需要推送时：-Push（并可用 -Tag 同时推送标签）。'
} else {
    if ($Branch -ne 'master') {
        Fail ("当前分支是 " + $Branch + "，不是 master：拒绝推送。") @(
            '发版请在 master 上执行：git switch master（或 git checkout master）后重跑。'
        )
    }

    Write-Info '先探测远端连通性：git ls-remote origin'
    $probe = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'ls-remote', 'origin')
    if ($probe.ExitCode -ne 0) {
        Fail '无法访问远端 origin（SSH）。' @(
            $probe.Text,
            ('本脚本使用的 GIT_SSH_COMMAND = ' + $env:GIT_SSH_COMMAND),
            '排查：ssh -T git@github.com ；国内网络可改用 Gitee 镜像推送（见 docs\发版流程.md）。'
        )
    }
    Write-Ok '远端连通（origin 可访问）'

    Invoke-NativeStream -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'push', 'origin', 'master') -What 'git push origin master'
    Write-Ok '已推送 master'

    if ($Tag) {
        $tagExistsCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'tag', '-l', $TagName)
        if ($tagExistsCap.Text.Trim().Length -gt 0) {
            Invoke-NativeStream -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'push', 'origin', ('refs/tags/' + $TagName)) -What ('git push origin refs/tags/' + $TagName)
            Write-Ok ('已推送标签：' + $TagName)
        } else {
            Write-Warn ('本地没有标签 ' + $TagName + '，未推送标签。')
        }
    }
    $Pushed = $true

    Write-Info '远端确认：git ls-remote origin（关键行）'
    $lsCap = Invoke-NativeCapture -FilePath 'git' -Arguments @('-C', $ProjectRoot, 'ls-remote', 'origin')
    if ($lsCap.ExitCode -ne 0) {
        Fail ('git ls-remote 失败：' + $lsCap.Text)
    }
    $keyLines = @($lsCap.Lines | Where-Object {
        $_.Contains('refs/heads/master') -or $_.Contains('refs/tags/' + $TagName)
    })
    if ($keyLines.Count -eq 0) {
        Write-Warn '没有匹配到 refs/heads/master 或标签行，请人工核对远端状态。'
    } else {
        foreach ($line in $keyLines) { Write-Ok ('远端 ' + $line) }
    }
}

# ---------------------------------------------------------------- 10/10 摘要

Write-Step -Number 10 -Text '发版摘要'

Write-Section '发版摘要'
Write-Host ('  版本号       : ' + $Version)
Write-Host ('  dist 产物    : ' + $DistExe)
Write-Host ('  versions 产物: ' + $VersionExe)
Write-Host ('  SHA256.txt   : ' + $ShaFile)
Write-Host ('  SHA256       : ' + $hashDist)
Write-Host ('  文件大小     : ' + (Get-Item -LiteralPath $DistExe).Length + ' 字节')
Write-Host ('  完整性标签   : ' + $(if ($ilDist.Level) { $ilDist.Level } else { '无显式标签(=Medium)' }) + '（来源 ' + $ilDist.SourcePath + '）')
Write-Host ('  提交         : ' + $HeadLine + '（本次' + $(if ($Committed) { '已提交' } else { '无新提交' }) + '）')
Write-Host ('  分支         : ' + $Branch)
if ($Tag) {
    Write-Host ('  标签         : ' + $TagName + $(if ($TagCreated) { '（本地新建）' } else { '（本地已存在）' }) +
                $(if ($Pushed) { '，已推送' } else { '，未推送' }))
}
Write-Host ('  推送         : ' + $(if ($Pushed) { '已完成（origin/master）' } else { '未推送（未加 -Push）' })) -ForegroundColor $(if ($Pushed) { 'Green' } else { 'Yellow' })
if ($BackupDir) { Write-Host ('  旧版备份     : ' + $BackupDir) }
Write-Host ('  耗时         : ' + [string]([int]((Get-Date) - $StartTime).TotalSeconds) + ' 秒')

Write-Section '手动待办（脚本不做的事）'
Write-Todo ('GitHub 网页建 Release 并上传附件：' + $VersionExe)
Write-Todo ('  入口：https://github.com/Xiuming-xm/XMST/releases/new?tag=' + $TagName)
Write-Todo '若尚未设置：仓库 Settings → Branches 把默认分支设为 master'
Write-Todo '  入口：https://github.com/Xiuming-xm/XMST/settings/branches'
if (-not $Pushed) {
    Write-Todo '本次未推送：确认无误后加 -Push 重跑（或用 git push origin master 手动推）'
}
Write-Todo '对外分发建议打成普通 zip（并提示用户用 7-Zip 解压，减少 MOTW/发布者提示）'

Write-Host ''
Write-Host ('发版流程结束。' + $(if ($Pushed) { '远端已更新。' } else { '仅本地完成（未推送）。' })) -ForegroundColor Green
exit 0
