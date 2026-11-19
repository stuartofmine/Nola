# SPDX-License-Identifier: Apache-2.0
# Exercise the installed release through Windows UI Automation with a fresh user profile.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System.Runtime.InteropServices;
public class DesktopInput {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, System.UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[void][DesktopInput]::SetProcessDPIAware()
$uia = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]::Descendants
$any = [System.Windows.Automation.Condition]::TrueCondition
function Find-Control([string]$property, [string]$value, [int]$seconds = 15) {
    $field = if ($property -eq 'id') { $uia::AutomationIdProperty } else { $uia::NameProperty }
    $match = New-Object System.Windows.Automation.PropertyCondition($field, $value)
    $end = [DateTime]::UtcNow.AddSeconds($seconds)
    do {
        $item = $script:window.FindFirst($scope, $match)
        if ($item -and !$item.Current.IsOffscreen) { return $item }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $end)
    $controls = $script:window.FindAll($scope, $any) | Where-Object { $_.Current.Name } | ForEach-Object { $_.Current.Name }
    throw "Control '$value' missing. Visible labels: $($controls -join ', ')"
}
function Click-Control($item) {
    $bounds = $item.Current.BoundingRectangle
    if ($bounds.IsEmpty) { throw 'Control has no click area' }
    [void][DesktopInput]::SetCursorPos([int]($bounds.X+$bounds.Width/2), [int]($bounds.Y+$bounds.Height/2))
    [DesktopInput]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    [DesktopInput]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
function Open-Desktop {
    $script:app = Start-Process $env:NOLA_INSTALLED_DESKTOP -PassThru
    $end = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $script:app.Refresh()
        if ($script:app.HasExited) { throw 'Installed desktop exited' }
        if ($script:app.MainWindowHandle -ne 0) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $end)
    $script:window = $uia::FromHandle($script:app.MainWindowHandle)
    $null = (New-Object -ComObject WScript.Shell).AppActivate($script:app.Id)
    $null = Find-Control name 'New conversation' 45
}
function Close-Desktop {
    if ($script:app -and !$script:app.HasExited) {
        $script:app.Kill()
        if (!$script:app.WaitForExit(10000)) { throw 'Installed desktop did not exit' }
    }
}
function Paste-Text([string]$value) {
    [Windows.Forms.Clipboard]::SetText($value)
    [Windows.Forms.SendKeys]::SendWait('^v')
}
function Draft-Value($control) {
    $control.SetFocus()
    [Windows.Forms.SendKeys]::SendWait('^a')
    [Windows.Forms.Clipboard]::Clear()
    [Windows.Forms.SendKeys]::SendWait('^c')
    Start-Sleep -Milliseconds 200
    return [Windows.Forms.Clipboard]::GetText()
}
if (!(Test-Path -LiteralPath $env:NOLA_INSTALLED_DESKTOP -PathType Leaf)) { throw 'Set NOLA_INSTALLED_DESKTOP to the installed executable' }
$env:NOLA_HOME = Join-Path ([IO.Path]::GetTempPath()) ('nola-desktop-acceptance-' + [Guid]::NewGuid())
$env:NOLA_E2E_USER_DATA = Join-Path $env:NOLA_HOME 'desktop'
$env:NOLA_DB_PATH = Join-Path $env:NOLA_HOME 'nola.db'
$null = New-Item -Type Directory -Path $env:NOLA_HOME
$note = Join-Path $env:NOLA_HOME 'restart-note.txt'
[IO.File]::WriteAllText($note, 'Unsent note')
$draft = 'Unsent desktop restart regression'
try {
    Open-Desktop
    $portFile = Join-Path $env:NOLA_E2E_USER_DATA 'host-port'
    $before = 'http://127.0.0.1:' + (Get-Content $portFile -Raw).Trim()
    Click-Control (Find-Control name 'New conversation')
    $editor = Find-Control id 'chat-composer'
    $editor.SetFocus()
    Paste-Text $draft
    if ((Draft-Value $editor) -ne $draft) { throw 'Native composer did not receive the draft' }
    $saved = @((Invoke-RestMethod "$before/api/chat/conversations").conversations)
    if ($saved.Count -ne 1 -or !$saved[0].conversationId) { throw 'Conversation creation did not commit' }
    Click-Control (Find-Control name 'Add text files')
    Start-Sleep -Milliseconds 900
    Paste-Text $note
    [Windows.Forms.SendKeys]::SendWait('{ENTER}')
    $null = Find-Control name 'restart-note.txt'
    Start-Sleep -Milliseconds 800
    Close-Desktop
    Open-Desktop
    $after = 'http://127.0.0.1:' + (Get-Content $portFile -Raw).Trim()
    if ($before -ne $after) { throw 'Desktop origin changed after restart' }
    $restored = @((Invoke-RestMethod "$after/api/chat/conversations").conversations)
    if ($restored.Count -ne 1 -or $restored[0].conversationId -ne $saved[0].conversationId) { throw 'Conversation changed after restart' }
    try { $editor = Find-Control id 'chat-composer' 2 }
    catch {
        $title = $restored[0].title
        if (!$title) { $title = 'Untitled conversation' }
        Click-Control (Find-Control name $title)
        $editor = Find-Control id 'chat-composer'
    }
    $actual = Draft-Value $editor
    if ($actual -ne $draft) { throw 'Unsent draft did not survive restart' }
    $attached = (Find-Control name 'restart-note.txt').Current.Name
    if ($env:NOLA_DESKTOP_RESTART_EVIDENCE) {
        $proof = @{platform='win32';beforeOrigin=$before;afterOrigin=$after;draft=$actual;attachment=$attached}
        [IO.File]::WriteAllText($env:NOLA_DESKTOP_RESTART_EVIDENCE, ($proof | ConvertTo-Json))
    }
    Write-Host 'Installed WebView2 passed conversation creation, draft entry, file attachment and restart.'
} finally { Close-Desktop }
