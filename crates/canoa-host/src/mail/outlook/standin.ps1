# A stand-in for Outlook's object model, for tests: the helper's own functions run against it unchanged. It holds a small
# mailbox in memory, keeps what a call changed in the file $script:StatePath names (each call is a new process), and
# understands the DASL and Jet filters the helper writes. Its Send throws, so a helper that sent would fail the test.

class SPerson {
    [string]$Name
    [string]$Address
    [string]$Type = 'SMTP'
    [int]$TypeNo = 1
}

class SRecipients : System.Collections.ArrayList {
    [bool] ResolveAll() { return $true }
}

class SRecipient {
    [string]$Name
    [int]$Type
    [object]$AddressEntry
}

class SAttachment {
    [string]$FileName = 'plan.pdf'
    [long]$Size = 2048
    [object]$PropertyAccessor
}

class SAccessor {
    # The Internet message id of the message this belongs to; a draft has none until it is sent.
    [string]$MessageId = ''
    [string] GetProperty([string]$tag) {
        if ($tag -like '*370E001E') { return 'application/pdf' }
        if ($tag -like '*1035001E') { return $this.MessageId }
        throw 'no such property'
    }
}

class SItems : System.Collections.ArrayList {
    [object] Restrict([string]$filter) {
        $kept = New-Object SItems
        foreach ($m in $this) {
            if (Test-StandinFilter $m $filter) { [void]$kept.Add($m) }
        }
        return $kept
    }
    [void] Sort([string]$by, [bool]$descending) {
        $sorted = @($this | Sort-Object { $_.ReceivedTime } -Descending:$descending)
        $this.Clear()
        foreach ($m in $sorted) { [void]$this.Add($m) }
    }
}

class SFolder {
    [string]$Name
    [string]$EntryID
    [string]$StoreID = 'STORE1'
    [int]$DefaultItemType = 0
    [System.Collections.ArrayList]$Folders = (New-Object System.Collections.ArrayList)
    [SItems]$Items = (New-Object SItems)
}

class SMail {
    [string]$EntryID
    [object]$Parent
    [string]$ConversationID
    [string]$Subject
    [string]$SenderName
    [string]$SenderEmailAddress
    [string]$SenderEmailType = 'SMTP'
    [object]$Sender
    [string]$To = ''
    [string]$CC = ''
    [string]$BCC = ''
    [datetime]$ReceivedTime = [datetime]'2026-11-01T00:00:00Z'
    [bool]$UnRead = $false
    [System.Collections.ArrayList]$Attachments = (New-Object System.Collections.ArrayList)
    [int]$FlagStatus = 0
    [string]$Categories = ''
    [string]$Body = ''
    [string]$HTMLBody = ''
    [int]$BodyFormat = 1
    [SRecipients]$Recipients = (New-Object SRecipients)
    [object]$PropertyAccessor = (New-Object SAccessor)

    [void] Send() { throw 'SEND WAS CALLED' }

    [void] Save() {
        if (-not $this.Parent) { $this.Parent = $script:Standin.Drafts }
        if (-not $this.EntryID) { $this.EntryID = 'ENTRY-' + ([guid]::NewGuid().ToString('N').Substring(0, 8)) }
        if (-not $this.Parent.Items.Contains($this)) { [void]$this.Parent.Items.Add($this) }
        if ($this.Recipients.Count -eq 0) {
            foreach ($spec in @(@('To', 1), @('CC', 2), @('BCC', 3))) {
                $text = [string]$this.($spec[0])
                foreach ($address in @($text -split ';' | Where-Object { $_ })) {
                    $p = New-Object SPerson
                    $p.Name = $address
                    $p.Address = $address
                    $r = New-Object SRecipient
                    $r.Name = $address
                    $r.Type = $spec[1]
                    $r.AddressEntry = $p
                    [void]$this.Recipients.Add($r)
                }
            }
        }
        Save-Standin
    }

    [object] Move([object]$folder) {
        [void]$this.Parent.Items.Remove($this)
        $this.Parent = $folder
        [void]$folder.Items.Add($this)
        Save-Standin
        return $this
    }

    [object] Reply() {
        $r = New-Object SMail
        $r.Subject = 'RE: ' + $this.Subject
        $r.To = $this.SenderEmailAddress
        $r.ConversationID = $this.ConversationID
        $r.Body = "`r`n`r`n> " + $this.Body
        return $r
    }

    [object] ReplyAll() { return $this.Reply() }

    [object] GetConversation() {
        $c = New-Object SConversation
        $c.Items = @($script:Standin.All() | Where-Object { $_.ConversationID -eq $this.ConversationID } | Sort-Object { $_.ReceivedTime })
        return $c
    }
}

class SConversation {
    [object[]]$Items
    [object[]] GetRootItems() { return @($this.Items | Select-Object -First 1) }
    [object[]] GetChildren([object]$parent) {
        $at = [array]::IndexOf($this.Items, $parent)
        if ($at -ge 0 -and $at + 1 -lt $this.Items.Count) { return @($this.Items[$at + 1]) }
        return @()
    }
}

class SNamespace {
    [object[]]$Folders
    [object] GetDefaultFolder([int]$n) {
        $name = switch ($n) { 6 { 'Inbox' } 5 { 'Sent Items' } 16 { 'Drafts' } 3 { 'Deleted Items' } 23 { 'Junk Email' } 4 { 'Outbox' } default { throw 'no such default folder' } }
        return $script:Standin.Find($name)
    }
    [object] GetFolderFromID([string]$id, [string]$store) { return $script:Standin.FindById($id) }
    [object] GetItemFromID([string]$id, [string]$store) {
        $found = @($script:Standin.All() | Where-Object { $_.EntryID -eq $id })
        if ($found.Count -eq 0) { throw 'The operation failed. The object cannot be found.' }
        return $found[0]
    }
    [object] GetItemFromID([string]$id) { return $this.GetItemFromID($id, '') }
}

class SApp {
    [object]$Ns
    [object] GetNamespace([string]$name) { return $this.Ns }
    [object] CreateItem([int]$kind) { return (New-Object SMail) }
}

class SStandin {
    [System.Collections.ArrayList]$Flat = (New-Object System.Collections.ArrayList)
    [object]$Root
    [object]$Drafts
    [object] Find([string]$name) { return @($this.Flat | Where-Object { $_.Name -eq $name })[0] }
    [object] FindById([string]$id) { return @($this.Flat | Where-Object { $_.EntryID -eq $id })[0] }
    [object[]] All() { return @($this.Flat | ForEach-Object { $_.Items }) }
}

function Test-StandinFilter($m, [string]$filter) {
    if ($filter.StartsWith('@SQL=')) {
        $e = $filter.Substring(5)
        $props = @{
            'urn:schemas:httpmail:subject' = '$m.Subject'
            'urn:schemas:httpmail:textdescription' = '$m.Body'
            'urn:schemas:httpmail:fromemail' = '$m.SenderEmailAddress'
            'urn:schemas:httpmail:fromname' = '$m.SenderName'
            'urn:schemas:httpmail:displayto' = '$m.To'
            'urn:schemas:httpmail:to' = '$m.To'
        }
        $e = [regex]::Replace($e, '"([^"]+)" LIKE ''%((?:[^'']|'''')*)%''', {
            param($mt)
            $prop = $props[$mt.Groups[1].Value]
            if (-not $prop) { throw ('the stand-in cannot read ' + $mt.Groups[1].Value) }
            '(' + $prop + " -like '*" + $mt.Groups[2].Value + "*')"
        })
        $e = [regex]::Replace($e, '"http://schemas.microsoft.com/mapi/proptag/0x1035001E" = ''((?:[^'']|'''')*)''', {
            param($mt)
            '($m.PropertyAccessor.GetProperty(''http://schemas.microsoft.com/mapi/proptag/0x1035001E'') -ceq ''' + $mt.Groups[1].Value + ''')'
        })
        $e = $e -replace '"urn:schemas:httpmail:hasattachment" = (\d)', '([int]($m.Attachments.Count -gt 0) -eq $1)'
        $e = $e -replace '"urn:schemas:httpmail:read" = (\d)', '([int](-not $m.UnRead) -eq $1)'
        $e = $e -replace '"http://schemas.microsoft.com/mapi/proptag/0x10900003" = 2', '($m.FlagStatus -eq 2)'
        $e = $e -replace '"http://schemas.microsoft.com/mapi/proptag/0x10900003" <> 2', '($m.FlagStatus -ne 2)'
        $e = $e -replace ' AND ', ' -and ' -replace ' OR ', ' -or '
        if ($e -match '"') { throw ('the stand-in cannot read the filter ' + $filter) }
        return [bool](Invoke-Expression $e)
    }
    $us = [Globalization.CultureInfo]::GetCultureInfo('en-US')
    $ok = $true
    foreach ($term in ($filter -split ' AND ')) {
        if ($term -match "^\[ReceivedTime\] (>=|<=) '([^']+)'$") {
            $when = [datetime]::Parse($Matches[2], $us)
            if ($Matches[1] -eq '>=') { $ok = $ok -and ($m.ReceivedTime.ToUniversalTime() -ge $when.ToUniversalTime()) }
            else { $ok = $ok -and ($m.ReceivedTime.ToUniversalTime() -le $when.ToUniversalTime()) }
        } else { throw ('the stand-in cannot read the filter ' + $term) }
    }
    return $ok
}

function Save-Standin {
    $rows = foreach ($m in $script:Standin.All()) {
        [ordered]@{
            id = $m.EntryID; folder = $m.Parent.Name; conversation = $m.ConversationID; subject = $m.Subject
            from_name = $m.SenderName; from = $m.SenderEmailAddress; to = $m.To
            received = $m.ReceivedTime.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
            unread = $m.UnRead; attachments = $m.Attachments.Count; flag = $m.FlagStatus
            categories = $m.Categories; body = $m.Body
        }
    }
    $json = ConvertTo-Json -InputObject @($rows) -Depth 4
    [IO.File]::WriteAllText($script:StatePath, $json, (New-Object Text.UTF8Encoding($false)))
}

function New-Folder($name, $id, $parent, $kind) {
    $f = New-Object SFolder
    $f.Name = $name
    $f.EntryID = $id
    $f.DefaultItemType = $kind
    [void]$script:Standin.Flat.Add($f)
    if ($parent) { [void]$parent.Folders.Add($f) }
    return $f
}

function New-OutlookApplication {
    $script:Standin = New-Object SStandin
    $root = New-Folder 'standin@example.test' 'F-root' $null 0
    $inbox = New-Folder 'Inbox' 'F-inbox' $root 0
    $sent = New-Folder 'Sent Items' 'F-sent' $root 0
    $script:Standin.Drafts = New-Folder 'Drafts' 'F-drafts' $root 0
    [void](New-Folder 'Deleted Items' 'F-deleted' $root 0)
    $archive = New-Folder 'Archive' 'F-archive' $root 0
    $projects = New-Folder 'Projects' 'F-projects' $root 0
    $apollo = New-Folder 'Apollo' 'F-apollo' $projects 0
    [void](New-Folder 'Calendar' 'F-calendar' $root 1)
    $byName = @{ 'Inbox' = $inbox; 'Sent Items' = $sent; 'Archive' = $archive; 'Drafts' = $script:Standin.Drafts; 'Apollo' = $apollo; 'Projects' = $projects }
    $rows = @()
    if (Test-Path $script:StatePath) {
        # Windows PowerShell 5.1 hands a JSON array on as one object: piping it through ForEach-Object lists its rows.
        $loaded = Get-Content -Raw -Encoding UTF8 $script:StatePath | ConvertFrom-Json
        $rows = @($loaded | ForEach-Object { $_ })
    } else {
        $seed = @(
            @('M1', 'Inbox', 'C1', 'Budget 2027 kickoff', 'Ana Ruiz', 'ana@example.test', 'me@example.test', '2026-11-10T09:00:00Z', $true, 1, "Let's plan the 2027 budget. The cap is 120k."),
            @('M2', 'Sent Items', 'C1', 'RE: Budget 2027 kickoff', 'Me', 'me@example.test', 'ana@example.test', '2026-11-10T11:00:00Z', $false, 0, 'Agreed, I will draft the numbers by Friday.'),
            @('M3', 'Inbox', 'C1', 'Re: Budget 2027 kickoff', 'Ana Ruiz', 'ana@example.test', 'me@example.test', '2026-11-11T08:00:00Z', $true, 0, 'Friday works. Include travel.'),
            @('M4', 'Inbox', 'C2', 'Budget 2027 kickoff', 'Bob Stone', 'bob@example.test', 'me@example.test', '2026-10-01T10:00:00Z', $false, 0, 'A different thread with the same title.'),
            @('M5', 'Inbox', 'C3', 'Lunch on Thursday?', 'Ana Ruiz', 'ana@example.test', 'me@example.test', '2026-11-12T12:00:00Z', $true, 0, 'Noon at the usual place?'),
            @('M6', 'Archive', 'C4', 'Invoice 4471', 'Carla', 'carla@vendor.test', 'me@example.test', '2025-03-02T10:00:00Z', $false, 1, 'Invoice attached.')
        )
        $rows = foreach ($s in $seed) {
            [pscustomobject]@{ id = $s[0]; folder = $s[1]; conversation = $s[2]; subject = $s[3]; from_name = $s[4]; from = $s[5]; to = $s[6]; received = $s[7]; unread = $s[8]; attachments = $s[9]; flag = 0; categories = ''; body = $s[10] }
        }
    }
    foreach ($row in $rows) {
        $m = New-Object SMail
        $m.EntryID = [string]$row.id
        $m.PropertyAccessor.MessageId = '<' + $m.EntryID + '@example.test>'
        $m.ConversationID = [string]$row.conversation
        $m.Subject = [string]$row.subject
        $m.SenderName = [string]$row.from_name
        $m.SenderEmailAddress = [string]$row.from
        $p = New-Object SPerson
        $p.Name = [string]$row.from_name
        $p.Address = [string]$row.from
        $m.Sender = $p
        $m.To = [string]$row.to
        # Windows PowerShell 5.1 turns an ISO date in the state file into a DateTime on its own.
        $m.ReceivedTime = ([datetime]$row.received).ToUniversalTime()
        $m.UnRead = [bool]$row.unread
        for ($i = 0; $i -lt [int]$row.attachments; $i++) {
            $a = New-Object SAttachment
            $a.PropertyAccessor = New-Object SAccessor
            [void]$m.Attachments.Add($a)
        }
        $m.FlagStatus = [int]$row.flag
        $m.Categories = [string]$row.categories
        $m.Body = [string]$row.body
        $recipient = New-Object SRecipient
        $recipient.Name = $m.To
        $recipient.Type = 1
        $person = New-Object SPerson
        $person.Name = $m.To
        $person.Address = $m.To
        $recipient.AddressEntry = $person
        [void]$m.Recipients.Add($recipient)
        $where = $byName[[string]$row.folder]
        $m.Parent = $where
        [void]$where.Items.Add($m)
    }
    $ns = New-Object SNamespace
    $ns.Folders = @($root)
    $app = New-Object SApp
    $app.Ns = $ns
    return $app
}
