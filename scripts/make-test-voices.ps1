# Synthetic German meeting voices (Windows OneCore TTS) for speaker tests.
# Saved with a UTF-8 BOM: Windows PowerShell 5 reads files without one as
# ANSI and the voices would then read out mangled umlauts.
param([string]$Out = 'E:\OwnApps\_agent\tts')
New-Item -ItemType Directory -Force -Path $Out | Out-Null
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, [Type]$type) {
    $task = $asTaskGeneric.MakeGenericMethod($type).Invoke($null, @($op))
    $task.Wait(-1) | Out-Null
    $task.Result
}
[Windows.Media.SpeechSynthesis.SpeechSynthesizer, Windows.Media.SpeechSynthesis, ContentType = WindowsRuntime] | Out-Null
$synth = New-Object Windows.Media.SpeechSynthesis.SpeechSynthesizer
$voices = @{}
foreach ($v in [Windows.Media.SpeechSynthesis.SpeechSynthesizer]::AllVoices) {
    if ($v.DisplayName -match 'Hedda') { $voices['hedda'] = $v }
    if ($v.DisplayName -match 'Katja') { $voices['katja'] = $v }
    if ($v.DisplayName -match 'Stefan') { $voices['stefan'] = $v }
}
$lines = @(
    @('hedda', 'Guten Morgen zusammen. Schön, dass ihr alle da seid. Heute geht es um die Planung für das nächste Quartal.'),
    @('stefan', 'Morgen. Bevor wir anfangen, eine kurze Frage zum Budget für die neuen Server. Ist das schon freigegeben?'),
    @('katja', 'Ja, die Finanzabteilung hat das letzte Woche bestätigt. Wir können die Hardware bestellen, sobald das Angebot da ist.'),
    @('hedda', 'Sehr gut. Dann kann das Team im März mit der Umstellung beginnen und die Tests bis Ende April abschließen.'),
    @('stefan', 'Das passt für mich. Ich kümmere mich um die Abstimmung mit dem Betriebsrat und den Datenschutz.'),
    @('katja', 'Denkt bitte auch an die Schulungen für die Kolleginnen und Kollegen in der Buchhaltung, die brauchen etwas mehr Zeit.'),
    @('hedda', 'Guter Punkt. Kannst du dafür bis Freitag einen kurzen Zeitplan schicken, damit ich ihn an die Geschäftsführung weitergeben kann?'),
    @('katja', 'Mache ich. Ich nehme auch die Risiken auf, über die wir gesprochen haben, und einen Plan für den Fall, dass die Lieferung später kommt.'),
    @('stefan', 'Dann sind wir für heute durch. Danke euch und bis nächste Woche.')
)
$second = @(
    @('stefan', 'Hallo, ich habe die Rückmeldung vom Datenschutz bekommen. Die Aufnahmen bleiben lokal, das ist so in Ordnung.'),
    @('hedda', 'Prima, dann können wir die Einführung wie geplant vorbereiten. Gibt es noch offene Fragen aus der Buchhaltung?'),
    @('katja', 'Zwei Kolleginnen wünschen sich eine zusätzliche Schulung am Nachmittag. Ich würde dafür den Mittwoch vorschlagen.'),
    @('stefan', 'Mittwoch geht bei mir. Ich reserviere den großen Besprechungsraum und schicke die Einladung heute noch raus.'),
    @('hedda', 'Danke. Dann halten wir fest: Schulung am Mittwoch, Einführung Anfang Mai, nächstes Treffen in zwei Wochen.'),
    @('katja', 'Alles klar, ich schreibe das Protokoll und lege es in den Projektordner.')
)
function Speak($list, $prefix) {
    $index = 0
    foreach ($line in $list) {
        $synth.Voice = $voices[$line[0]]
        $stream = Await ($synth.SynthesizeTextToStreamAsync($line[1])) ([Windows.Media.SpeechSynthesis.SpeechSynthesisStream])
        $read = [System.IO.WindowsRuntimeStreamExtensions]::AsStreamForRead($stream)
        $path = Join-Path $Out ('{0}-{1:D2}-{2}.wav' -f $prefix, $index, $line[0])
        $file = [System.IO.File]::Create($path)
        $read.CopyTo($file)
        $file.Close()
        $index++
    }
}
Speak $lines 'a'
Speak $second 'b'
Get-ChildItem $Out | Select-Object Name, Length | Format-Table -AutoSize | Out-String -Width 120
