# aIwalk System Setup: Windows VM disk cleanup - run as Administrator
# ASCII only: Windows PowerShell 5.1 reads .ps1 as ANSI/CP950, non-ASCII breaks parsing.
$ErrorActionPreference = 'SilentlyContinue'

function Show-Disk($label) {
    $d = Get-PSDrive C
    "{0,-8} Used {1,6:N1} GB   Free {2,6:N1} GB" -f $label, ($d.Used/1GB), ($d.Free/1GB)
}

Write-Host ""
Write-Host (Show-Disk "BEFORE") -ForegroundColor Cyan
Write-Host ""

Write-Host "1/9 kernel crash dumps" -ForegroundColor Yellow
Remove-Item 'C:\Windows\LiveKernelReports\*.dmp' -Force
Remove-Item 'C:\Windows\LiveKernelReports\*\*' -Recurse -Force
Remove-Item 'C:\Windows\Minidump\*' -Recurse -Force
Remove-Item 'C:\Windows\MEMORY.DMP' -Force

Write-Host "2/9 PowerPoint autorecover cache" -ForegroundColor Yellow
Remove-Item "$env:APPDATA\Microsoft\PowerPoint\*" -Recurse -Force

Write-Host "3/9 Office document cache" -ForegroundColor Yellow
Remove-Item "$env:LOCALAPPDATA\Microsoft\Office\16.0\OfficeFileCache\*" -Recurse -Force

Write-Host "4/9 LINE cache" -ForegroundColor Yellow
Remove-Item "$env:LOCALAPPDATA\LINE\Cache\*" -Recurse -Force

Write-Host "5/9 temp + windows update download cache" -ForegroundColor Yellow
Remove-Item "$env:TEMP\*" -Recurse -Force
Remove-Item 'C:\Windows\Temp\*' -Recurse -Force
Remove-Item 'C:\Windows\SoftwareDistribution\Download\*' -Recurse -Force

Write-Host "6/9 recycle bin" -ForegroundColor Yellow
Clear-RecycleBin -Force -Confirm:$false

Write-Host "7/9 disable hibernation, pin pagefile to 1 GB" -ForegroundColor Yellow
powercfg /h off
$cs = Get-WmiObject Win32_ComputerSystem -EnableAllPrivileges
$cs.AutomaticManagedPagefile = $false
$cs.Put() | Out-Null
$pf = Get-WmiObject Win32_PageFileSetting
if ($pf) { $pf.InitialSize = 1024; $pf.MaximumSize = 1024; $pf.Put() | Out-Null }

Write-Host "8/9 WinSxS component cleanup (slow, 5-10 min)" -ForegroundColor Yellow
Dism /Online /Cleanup-Image /StartComponentCleanup /ResetBase

Write-Host "9/9 ReTrim - this is what shrinks the host image file" -ForegroundColor Yellow
Optimize-Volume -DriveLetter C -ReTrim -Verbose

Write-Host ""
Write-Host (Show-Disk "AFTER") -ForegroundColor Cyan
Write-Host ""

Write-Host "Leftover installers in Downloads (delete these yourself if you want):" -ForegroundColor Magenta
Get-ChildItem "$env:USERPROFILE\Downloads" -Force |
    Sort-Object Length -Descending |
    Select-Object -First 10 Name, @{n='MB'; e={ [math]::Round($_.Length/1MB) }} |
    Format-Table -AutoSize

Write-Host ""
Write-Host "DONE. Now shut down Windows: Start -> Power -> Shut down." -ForegroundColor Green
Read-Host "Press Enter to exit"
