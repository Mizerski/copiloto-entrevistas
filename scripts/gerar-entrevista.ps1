# Gera uma fala em WAV (mono, 16 kHz) por linha de um roteiro de entrevista.
# Linhas "E: ..." sao do entrevistador e "C: ..." do candidato. Os arquivos saem na
# mesma pasta do roteiro, numerados: 001-E.wav, 002-C.wav... O montar_entrevista junta.
#
#   .\scripts\gerar-entrevista.ps1 testes\entrevista-1\roteiro.txt
param(
    [Parameter(Mandatory = $true)][string]$Roteiro,
    [string]$Voz = "Microsoft Maria Desktop"
)

Add-Type -AssemblyName System.Speech
$entrada = Resolve-Path $Roteiro
$pasta = Split-Path $entrada
Get-ChildItem $pasta -Filter "???-?.wav" | Remove-Item
$formato = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$n = 0
foreach ($linha in Get-Content $entrada -Encoding UTF8) {
    if ($linha -notmatch '^\s*([EC]):\s*(.+)$') { continue }
    $n++
    $saida = Join-Path $pasta ("{0:D3}-{1}.wav" -f $n, $Matches[1])
    $sintetizador = New-Object System.Speech.Synthesis.SpeechSynthesizer
    $sintetizador.SelectVoice($Voz)
    $sintetizador.SetOutputToWaveFile($saida, $formato)
    $sintetizador.Speak($Matches[2].Trim())
    $sintetizador.Dispose()
}
Write-Output "Geradas $n falas em $pasta"
