#requires -Version 5.1
# The helper the host runs to reach classic Outlook for Windows through its object model (COM). It reads one JSON request
# (base64, as the argument of Invoke-CanoaMail), does it as the person signed in to Outlook, and writes one JSON answer to
# standard output: {"ok":true,"result":...} or {"ok":false,"error":"..."}.
#
# It never sends: nothing here calls a MailItem's Send, and a draft is only saved. Outlook may ask the person to allow
# programmatic access (an antivirus product or a policy decides); the call then waits for the answer.

$ErrorActionPreference = 'Stop'

# The default folder numbers of Outlook's OlDefaultFolders.
$script:Well = @{
    inbox = 6; sent = 5; sentitems = 5; drafts = 16; draft = 16; deleted = 3; deleteditems = 3; trash = 3
    junk = 23; junkemail = 23; spam = 23; outbox = 4
}

function New-OutlookApplication {
    New-Object -ComObject Outlook.Application
}

function Get-Smtp($entry) {
    # An Exchange address is a directory name; the mailbox's SMTP address is behind the address entry.
    try {
        if ($entry -and $entry.Type -eq 'EX') {
            $user = $entry.GetExchangeUser()
            if ($user -and $user.PrimarySmtpAddress) { return [string]$user.PrimarySmtpAddress }
        }
    } catch { }
    try { if ($entry) { return [string]$entry.Address } } catch { }
    return ''
}

function Get-Person($name, $address) {
    return @{ name = [string]$name; address = [string]$address }
}

function Get-Recipients($item, $type) {
    $found = @()
    foreach ($r in $item.Recipients) {
        if ($r.Type -eq $type) {
            $found += , (Get-Person $r.Name (Get-Smtp $r.AddressEntry))
        }
    }
    return , $found
}

function Get-ItemId($item) {
    return ([string]$item.EntryID) + '|' + ([string]$item.Parent.StoreID)
}

function Get-Summary($item) {
    $from = $null
    try {
        $address = [string]$item.SenderEmailAddress
        if ($item.SenderEmailType -eq 'EX') { $address = Get-Smtp $item.Sender }
        $from = Get-Person $item.SenderName $address
    } catch { }
    $received = $null
    try { $received = ([datetime]$item.ReceivedTime).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') } catch { }
    $categories = @()
    if ($item.Categories) { $categories = @(([string]$item.Categories) -split '\s*[,;]\s*' | Where-Object { $_ }) }
    $preview = ''
    try { $preview = ([string]$item.Body); if ($preview.Length -gt 300) { $preview = $preview.Substring(0, 300) } } catch { }
    return @{
        id = Get-ItemId $item
        conversation = [string]$item.ConversationID
        folder = [string]$item.Parent.EntryID
        subject = [string]$item.Subject
        from = $from
        to = (Get-Recipients $item 1)
        cc = (Get-Recipients $item 2)
        received = $received
        unread = [bool]$item.UnRead
        attachments = ($item.Attachments.Count -gt 0)
        flagged = ($item.FlagStatus -eq 2)
        categories = $categories
        preview = $preview
    }
}

function Get-FolderId($folder) {
    return ([string]$folder.EntryID) + '|' + ([string]$folder.StoreID)
}

# Every mail folder of every store, with its path from the store's name.
function Get-AllFolders($ns) {
    $found = New-Object System.Collections.ArrayList
    $walk = {
        param($folder, $path)
        $here = if ($path) { $path + '/' + $folder.Name } else { [string]$folder.Name }
        # 0 is olMailItem: calendars, contacts and the rest are not mail.
        if ($folder.DefaultItemType -eq 0) { [void]$found.Add(@{ folder = $folder; path = $here }) }
        foreach ($child in $folder.Folders) { & $walk $child $here }
    }
    foreach ($root in $ns.Folders) { & $walk $root '' }
    return , $found
}

function Resolve-Folder($ns, [string]$name, $all) {
    $key = $name.Trim().ToLower()
    if ($script:Well.ContainsKey($key)) { return $ns.GetDefaultFolder($script:Well[$key]) }
    if ($key -eq 'archive') {
        $named = @($all | Where-Object { $_.folder.Name -eq 'Archive' })
        if ($named.Count -gt 0) { return $named[0].folder }
        throw 'the mailbox has no Archive folder: move the messages to a folder named by mail_list'
    }
    if ($name -like '*|*') {
        $parts = $name -split '\|', 2
        return $ns.GetFolderFromID($parts[0], $parts[1])
    }
    $wanted = $key.Trim('/')
    $byPath = @($all | Where-Object { $_.path.ToLower() -eq $wanted -or $_.path.ToLower().EndsWith('/' + $wanted) })
    if ($byPath.Count -eq 1) { return $byPath[0].folder }
    if ($byPath.Count -gt 1) { throw ("$name is more than one folder (" + (($byPath | ForEach-Object { $_.path }) -join ', ') + '): name it by its path') }
    throw "no folder is named ${name}: mail_list names the folders"
}

function Get-MailItem($ns, [string]$id) {
    $parts = $id -split '\|', 2
    if ($parts.Count -eq 2) { return $ns.GetItemFromID($parts[0], $parts[1]) }
    return $ns.GetItemFromID($parts[0])
}

function Escape-Dasl([string]$text) {
    return $text.Replace("'", "''")
}

function Like($property, [string]$text) {
    return ('"' + $property + '" LIKE ''%' + (Escape-Dasl $text) + '%''')
}

function Get-Filters($q) {
    $dasl = New-Object System.Collections.ArrayList
    $mail = 'urn:schemas:httpmail:'
    if ($q.from) {
        [void]$dasl.Add('(' + (Like ($mail + 'fromemail') $q.from) + ' OR ' + (Like ($mail + 'fromname') $q.from) + ')')
    }
    if ($q.to) {
        [void]$dasl.Add('(' + (Like ($mail + 'displayto') $q.to) + ' OR ' + (Like ($mail + 'to') $q.to) + ')')
    }
    if ($q.subject) { [void]$dasl.Add((Like ($mail + 'subject') $q.subject)) }
    if ($q.text) {
        # Words must all appear in the subject or the body; OR between groups of words is kept.
        $groups = New-Object System.Collections.ArrayList
        foreach ($alternative in @(([string]$q.text) -split '\s+OR\s+')) {
            $words = @($alternative -split '\s+' | ForEach-Object { $_.Trim('()"') } | Where-Object { $_ -and $_ -ne 'AND' })
            if ($words.Count -gt 0) {
                $each = @($words | ForEach-Object { '(' + (Like ($mail + 'subject') $_) + ' OR ' + (Like ($mail + 'textdescription') $_) + ')' })
                [void]$groups.Add('(' + ($each -join ' AND ') + ')')
            }
        }
        if ($groups.Count -gt 0) { [void]$dasl.Add('(' + ($groups -join ' OR ') + ')') }
    }
    if ($null -ne $q.attachments) { [void]$dasl.Add('"' + $mail + 'hasattachment" = ' + $(if ($q.attachments) { 1 } else { 0 })) }
    if ($null -ne $q.unread) { [void]$dasl.Add('"' + $mail + 'read" = ' + $(if ($q.unread) { 0 } else { 1 })) }
    if ($null -ne $q.flagged) {
        $op = if ($q.flagged) { '=' } else { '<>' }
        [void]$dasl.Add('"http://schemas.microsoft.com/mapi/proptag/0x10900003" ' + $op + ' 2')
    }
    $jet = New-Object System.Collections.ArrayList
    $us = [Globalization.CultureInfo]::GetCultureInfo('en-US')
    if ($q.after) { [void]$jet.Add("[ReceivedTime] >= '" + ([datetime]$q.after).ToString('g', $us) + "'") }
    if ($q.before) { [void]$jet.Add("[ReceivedTime] <= '" + ([datetime]$q.before).AddDays(1).ToString('g', $us) + "'") }
    $daslText = if ($dasl.Count -gt 0) { '@SQL=' + ($dasl -join ' AND ') } else { $null }
    return @{ jet = $(if ($jet.Count -gt 0) { $jet -join ' AND ' } else { $null }); dasl = $daslText }
}

function Search-Mail($ns, $q) {
    $all = Get-AllFolders $ns
    $scope = @($all)
    $deep = $true
    if ($q.folder -and $q.folder -ne 'all' -and $q.folder -ne 'root' -and $q.folder -ne 'mailbox') {
        $one = Resolve-Folder $ns ([string]$q.folder) $all
        $scope = @(@{ folder = $one; path = $one.Name })
    }
    $filters = Get-Filters $q
    $want = [int]$q.offset + [int]$q.limit
    $rows = New-Object System.Collections.ArrayList
    $total = 0
    foreach ($entry in $scope) {
        $items = $entry.folder.Items
        if ($filters.jet) { $items = $items.Restrict($filters.jet) }
        if ($filters.dasl) { $items = $items.Restrict($filters.dasl) }
        $total += $items.Count
        $items.Sort('[ReceivedTime]', $true)
        $taken = 0
        foreach ($item in $items) {
            if ($taken -ge $want) { break }
            $taken++
            [void]$rows.Add((Get-Summary $item))
        }
    }
    $sorted = @($rows | Sort-Object { $_.received } -Descending)
    $page = @($sorted | Select-Object -Skip ([int]$q.offset) -First ([int]$q.limit))
    return @{ items = $page; total = $total; more = (([int]$q.offset + $page.Count) -lt $total) }
}

# The message whose Internet message id (PR_INTERNET_MESSAGE_ID) is $messageId, in any mail folder of any store.
function Find-ByMessageId($ns, [string]$messageId) {
    $dasl = '@SQL="http://schemas.microsoft.com/mapi/proptag/0x1035001E" = ''' + (Escape-Dasl $messageId) + ''''
    foreach ($entry in (Get-AllFolders $ns)) {
        foreach ($item in $entry.folder.Items.Restrict($dasl)) {
            return @{ id = (Get-ItemId $item) }
        }
    }
    return @{ id = '' }
}

function Get-Message($item) {
    $summary = Get-Summary $item
    $body = ''
    try { $body = [string]$item.Body } catch { }
    $attachments = @()
    foreach ($a in $item.Attachments) {
        $size = $null
        try { $size = [long]$a.Size } catch { }
        $type = $null
        try { $type = [string]$a.PropertyAccessor.GetProperty('http://schemas.microsoft.com/mapi/proptag/0x370E001E') } catch { }
        $attachments += , @{ name = [string]$a.FileName; size = $size; content_type = $type }
    }
    $message_id = $null
    try { $message_id = [string]$item.PropertyAccessor.GetProperty('http://schemas.microsoft.com/mapi/proptag/0x1035001E') } catch { }
    return @{ summary = $summary; body = $body; attachments = $attachments; message_id = $message_id }
}

function Get-Thread($ns, $item) {
    $conversation = $item.GetConversation()
    $found = New-Object System.Collections.ArrayList
    $walk = {
        param($parent)
        foreach ($child in $conversation.GetChildren($parent)) {
            [void]$found.Add($child)
            & $walk $child
        }
    }
    foreach ($root in $conversation.GetRootItems()) {
        [void]$found.Add($root)
        & $walk $root
    }
    return @($found | ForEach-Object { Get-Summary $_ })
}

function New-Draft($app, $ns, $d) {
    if ($d.reply_to) {
        $original = Get-MailItem $ns ([string]$d.reply_to)
        $draft = if ($d.reply_all) { $original.ReplyAll() } else { $original.Reply() }
        if ($draft.BodyFormat -eq 2) {
            $html = '<p>' + ([System.Net.WebUtility]::HtmlEncode([string]$d.body) -replace "`r?`n", '<br>') + '</p>'
            $draft.HTMLBody = [regex]::Replace($draft.HTMLBody, '(<body[^>]*>)', ('$1' + $html.Replace('$', '$$')), 'IgnoreCase')
        } else {
            $draft.Body = [string]$d.body + "`r`n`r`n" + $draft.Body
        }
    } else {
        $draft = $app.CreateItem(0)
        $draft.To = (@($d.to) -join ';')
        if ($d.cc) { $draft.CC = (@($d.cc) -join ';') }
        if ($d.bcc) { $draft.BCC = (@($d.bcc) -join ';') }
        $draft.Subject = [string]$d.subject
        $draft.Body = [string]$d.body
        [void]$draft.Recipients.ResolveAll()
    }
    $draft.Save()
    return Get-Summary $draft
}

function Set-Change($ns, $id, $change, $all) {
    $item = Get-MailItem $ns $id
    switch ($change.kind) {
        'move' { [void]$item.Move((Resolve-Folder $ns ([string]$change.folder) $all)) }
        'archive' { [void]$item.Move((Resolve-Folder $ns 'archive' $all)) }
        'read' { $item.UnRead = (-not $change.read); $item.Save() }
        'flag' {
            $item.FlagStatus = switch ([string]$change.flag) { 'flagged' { 2 } 'complete' { 1 } default { 0 } }
            $item.Save()
        }
        'categories' {
            $have = @()
            if ($item.Categories) { $have = @(([string]$item.Categories) -split '\s*[,;]\s*' | Where-Object { $_ }) }
            $remove = @($change.remove)
            $have = @($have | Where-Object { $name = $_; -not ($remove | Where-Object { $_ -ieq $name }) })
            foreach ($name in @($change.add)) {
                if (-not ($have | Where-Object { $_ -ieq $name })) { $have += $name }
            }
            $item.Categories = ($have -join ', ')
            $item.Save()
        }
        default { throw "no change is named $($change.kind)" }
    }
}

function Invoke-CanoaMail([string]$encoded) {
    try {
        $json = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($encoded))
        $request = $json | ConvertFrom-Json
        $app = New-OutlookApplication
        $ns = $app.GetNamespace('MAPI')
        if ($request.profile) { $ns.Logon([string]$request.profile, '', $false, $false) }
        $result = switch ($request.op) {
            'folders' {
                $all = Get-AllFolders $ns
                @($all | ForEach-Object {
                    $f = $_.folder
                    @{ id = Get-FolderId $f; name = [string]$f.Name; path = $_.path; total = [long]$f.Items.Count; unread = [long]$f.UnReadItemCount }
                })
            }
            'search' { Search-Mail $ns $request }
            'read' { Get-Message (Get-MailItem $ns ([string]$request.id)) }
            'find_by_message_id' { Find-ByMessageId $ns ([string]$request.message_id) }
            'summaries' { @(@($request.ids) | ForEach-Object { Get-Summary (Get-MailItem $ns ([string]$_)) }) }
            'thread' { Get-Thread $ns (Get-MailItem $ns ([string]$request.id)) }
            'draft' { New-Draft $app $ns $request }
            'change' {
                $all = Get-AllFolders $ns
                foreach ($id in @($request.ids)) { Set-Change $ns ([string]$id) $request.change $all }
                $true
            }
            default { throw "no operation is named $($request.op)" }
        }
        $answer = @{ ok = $true; result = $result }
    } catch {
        $answer = @{ ok = $false; error = [string]$_.Exception.Message }
    }
    $text = ConvertTo-Json -InputObject $answer -Depth 12 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($text)
    $out = [Console]::OpenStandardOutput()
    $out.Write($bytes, 0, $bytes.Length)
    $out.Flush()
}
