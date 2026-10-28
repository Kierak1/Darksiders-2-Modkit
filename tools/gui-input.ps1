param(
    [ValidateSet("click", "dblclick", "drag", "type", "wheel")]
    [string]$Cmd = "click",
    [int]$X = 0,
    [int]$Y = 0,
    [int]$X2 = 0,
    [int]$Y2 = 0,
    [string]$Text = "",
    [int]$Steps = 12,
    [int]$Wheel = 0,
    [string]$Proc = "ds2-modkit-gui"
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class GInput {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern void SwitchToThisWindow(IntPtr h, bool alt);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
Add-Type -AssemblyName System.Windows.Forms
$p = Get-Process $Proc -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Error "process '$Proc' not running"; exit 1 }
$h = $p.MainWindowHandle
if ($h -eq [IntPtr]::Zero) { Write-Error "no main window"; exit 1 }
[GInput]::SwitchToThisWindow($h, $true) | Out-Null
[GInput]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300
$r = New-Object GInput+RECT
[GInput]::GetWindowRect($h, [ref]$r) | Out-Null
$sx = $r.L + $X
$sy = $r.T + $Y
switch ($Cmd) {
    "click" {
        [GInput]::SetCursorPos($sx, $sy) | Out-Null
        Start-Sleep -Milliseconds 80
        [GInput]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 50
        [GInput]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    }
    "dblclick" {
        [GInput]::SetCursorPos($sx, $sy) | Out-Null
        Start-Sleep -Milliseconds 80
        1..2 | ForEach-Object {
            [GInput]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
            [GInput]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds 50
        }
    }
    "drag" {
        $ex = $r.L + $X2
        $ey = $r.T + $Y2
        [GInput]::SetCursorPos($sx, $sy) | Out-Null
        Start-Sleep -Milliseconds 80
        [GInput]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        for ($i = 1; $i -le $Steps; $i++) {
            $ix = [int]($sx + ($ex - $sx) * $i / $Steps)
            $iy = [int]($sy + ($ey - $sy) * $i / $Steps)
            [GInput]::SetCursorPos($ix, $iy) | Out-Null
            Start-Sleep -Milliseconds 20
        }
        [GInput]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    }
    "wheel" {
        [GInput]::SetCursorPos($sx, $sy) | Out-Null
        Start-Sleep -Milliseconds 60
        $delta = [uint32]($Wheel -shl 16)
        $d0 = $delta -band 0xFFFF
        $d1 = ($delta -shr 16) -band 0xFFFF
        $packed = [uint32](($d1 -shl 16) -bor $d0)
        [GInput]::mouse_event(0x0800, 0, 0, $packed, [UIntPtr]::Zero)
    }
    "type" {
        [System.Windows.Forms.SendKeys]::SendWait($Text)
    }
}
Write-Output "$Cmd window ($X,$Y) screen ($sx,$sy)"
