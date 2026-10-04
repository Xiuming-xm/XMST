# 以捕获 stdout/stderr 的方式启动 XMST，便于抓到崩溃输出。
# 用法：powershell -NoProfile -ExecutionPolicy Bypass -File F:\XMST\tools\run_with_capture.ps1
$exe = 'F:\XMST\dist\XMST-0.1.1-alpha.exe'
$out = 'F:\XMST\dist\data\ui_stderr.log'
New-Item -ItemType Directory -Force -Path 'F:\XMST\dist\data' | Out-Null
"===== 启动 $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') =====" | Set-Content -LiteralPath $out -Encoding UTF8
$p = Start-Process -FilePath $exe -RedirectStandardOutput 'F:\XMST\dist\data\ui_stdout.log' -RedirectStandardError $out -PassThru
"已启动 XMST (PID $($p.Id))。请复现：点击左侧『日志』。"
"崩溃或退出后，把下面这个文件发我： $out"
"也可直接告诉我它的最后 40 行。"
$p.WaitForExit()
"`n===== 进程已退出，退出码 $($p.ExitCode) =====" | Add-Content -LiteralPath $out -Encoding UTF8
"退出码: $($p.ExitCode)"