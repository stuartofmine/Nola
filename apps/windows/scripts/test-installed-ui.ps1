# SPDX-License-Identifier: Apache-2.0
# Exercise the installed release through Windows UI Automation with a fresh user profile.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System.Runtime.InteropServices;
public class DesktopInput {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, System.UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
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
# Keep only the foreground test window in the evidence, including the native
# picker when it owns the interaction. UIA supplies physical screen coordinates.
function Save-StepImage([string]$name, $element = $script:window) {
    if ([DesktopInput]::GetForegroundWindow().ToInt64() -ne $element.Current.NativeWindowHandle) {
        throw "Test window is not in front at $name"
    }
    $bounds = $element.Current.BoundingRectangle
    $rect = [Drawing.Rectangle]::FromLTRB([int]$bounds.Left, [int]$bounds.Top,
        [int]$bounds.Right, [int]$bounds.Bottom)
    $rect = [Drawing.Rectangle]::Intersect($rect, [Windows.Forms.SystemInformation]::VirtualScreen)
    if ($rect.Width -le 0 -or $rect.Height -le 0) { throw "Cannot capture $name" }
    $bitmap = New-Object Drawing.Bitmap($rect.Width, $rect.Height)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try { $graphics.CopyFromScreen($rect.Location, [Drawing.Point]::Empty, $rect.Size) }
        finally { $graphics.Dispose() }
        $bitmap.Save((Join-Path $script:images "$name.png"), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
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
    Write-Host ("Starting installed desktop, PID " + $script:app.Id)
    $match = New-Object System.Windows.Automation.PropertyCondition($uia::NameProperty, 'New conversation')
    do {
        $script:app.Refresh()
        if ($script:app.HasExited) { throw 'Installed desktop exited' }
        if ($script:app.MainWindowHandle -ne 0 -and $script:app.MainWindowTitle -like '*Nola*') {
            $script:window = $uia::FromHandle($script:app.MainWindowHandle)
            $null = (New-Object -ComObject WScript.Shell).AppActivate($script:app.Id)
            if ($script:window.FindFirst($scope, $match)) { return }
            [Windows.Forms.SendKeys]::SendWait('{TAB}')
        }
        Start-Sleep -Milliseconds 400
    } while ([DateTime]::UtcNow -lt $end)
    throw "Chat window unavailable after launch. Title: $($script:app.MainWindowTitle)"
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
    $pattern = $null
    if ($control.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) {
        return $pattern.Current.Value
    }
    if ($control.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern, [ref]$pattern)) {
        return $pattern.DocumentRange.GetText(-1).TrimEnd([char]13,[char]10)
    }
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
$imageRoot = if ($env:NOLA_UI_SCREENSHOTS) { $env:NOLA_UI_SCREENSHOTS } else { $env:NOLA_HOME }
$script:images = Join-Path $imageRoot ('restart-images-' + [Guid]::NewGuid())
$null = New-Item -Type Directory -Force -Path $script:images
Write-Host "Windows interaction screenshots: $script:images"
$note = Join-Path $env:NOLA_HOME 'restart-note.txt'
[IO.File]::WriteAllText($note, 'Unsent note')
$draft = 'Unsent desktop restart regression'
try {
    Open-Desktop
    $portFile = Join-Path $env:NOLA_E2E_USER_DATA 'host-port'
    $before = 'http://127.0.0.1:' + (Get-Content $portFile -Raw).Trim()
    Write-Host 'Creating conversation and entering a draft'
    Save-StepImage '01-before-create'
    Click-Control (Find-Control name 'New conversation')
    $editor = Find-Control id 'chat-composer'
    Save-StepImage '02-conversation-open'
    $editor.SetFocus()
    Paste-Text $draft
    if ((Draft-Value $editor) -ne $draft) { throw 'Native composer did not receive the draft' }
    $saved = @((Invoke-RestMethod "$before/api/chat/conversations").conversations)
    if ($saved.Count -ne 1 -or !$saved[0].conversationId) { throw 'Conversation creation did not commit' }
    Save-StepImage '03-draft-entered'
    $picker = Find-Control name 'Add text files: No file chosen'
    $picker.SetFocus()
    [Windows.Forms.SendKeys]::SendWait('{ENTER}')
    $dialog = $null
    for ($i = 0; $i -lt 50; $i++) {
        $foreground = $uia::FromHandle([DesktopInput]::GetForegroundWindow())
        if ($foreground.Current.ClassName -eq '#32770') { $dialog = $foreground; break }
        Start-Sleep -Milliseconds 200
    }
    if (!$dialog) {
        $focused = $uia::FocusedElement.Current
        throw "File picker unavailable. Foreground: $($foreground.Current.ClassName) / $($foreground.Current.Name). Focus: $($focused.ControlType.ProgrammaticName) / $($focused.Name)"
    }
    Save-StepImage '04-file-picker' $dialog
    [Windows.Forms.SendKeys]::SendWait('%n')
    [Windows.Forms.SendKeys]::SendWait('^a')
    Paste-Text $note
    [Windows.Forms.SendKeys]::SendWait('{ENTER}')
    $null = Find-Control name 'restart-note.txt'
    if ((Draft-Value (Find-Control id 'chat-composer')) -ne $draft) { throw 'Draft changed while selecting the attachment' }
    Save-StepImage '05-draft-and-attachment'
    Write-Host 'Confirmed draft and attachment before exit'
    Start-Sleep -Milliseconds 1200
    Close-Desktop
    Open-Desktop
    $after = 'http://127.0.0.1:' + (Get-Content $portFile -Raw).Trim()
    if ($before -ne $after) { throw 'Desktop origin changed after restart' }
    Write-Host 'Reopened installed desktop with the original Host port'
    $restored = @((Invoke-RestMethod "$after/api/chat/conversations").conversations)
    if ($restored.Count -ne 1 -or $restored[0].conversationId -ne $saved[0].conversationId) { throw 'Conversation changed after restart' }
    $title = $restored[0].title
    if (!$title) { $title = 'Untitled conversation' }
    $row = Find-Control name $title
    Save-StepImage '06-before-reopen-conversation'
    Click-Control $row
    $attached = (Find-Control name 'restart-note.txt').Current.Name
    $editor = Find-Control id 'chat-composer'
    $actual = Draft-Value $editor
    if ($actual -ne $draft) { throw "Restored draft differs, expected '$draft', received '$actual'" }
    Save-StepImage '07-restored-draft-and-attachment'
    if ($env:NOLA_REAL_LLM_E2E -eq '1') {
        $marker = 'WIN_' + [Guid]::NewGuid().ToString('N')
        $editor.SetFocus()
        [Windows.Forms.SendKeys]::SendWait('^a')
        Paste-Text "Reply with exactly $marker"
        Click-Control (Find-Control name 'Send')
        $deadline = [DateTime]::UtcNow.AddSeconds(90)
        do {
            $transcript = Invoke-RestMethod "$after/api/chat/conversations/$($saved[0].conversationId)/messages"
            $reply = $transcript.messages | Where-Object { $_.role -eq 'assistant' -and $_.content -like "*$marker*" }
            if ($reply) { break }
            Start-Sleep -Milliseconds 300
        } while ([DateTime]::UtcNow -lt $deadline)
        if (!$reply) { throw 'Installed desktop did not persist the expected model reply' }
        Save-StepImage '08-real-model-reply'
    }
    if ($env:NOLA_DESKTOP_RESTART_EVIDENCE) {
        $proof = @{platform='win32';beforeOrigin=$before;afterOrigin=$after;draft=$actual;attachment=$attached}
        [IO.File]::WriteAllText($env:NOLA_DESKTOP_RESTART_EVIDENCE, ($proof | ConvertTo-Json))
    }
    Write-Host 'Installed WebView2 passed conversation creation, draft entry, file attachment and restart.'
} catch {
    $failure = $_
    if ($script:app -and !$script:app.HasExited -and $script:window) {
        try { Save-StepImage 'failure' } catch { Write-Warning 'Could not capture the failed window' }
    }
    throw $failure
} finally { Close-Desktop }
