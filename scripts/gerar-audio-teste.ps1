# Gera um WAV de teste (mono, 16 kHz) a partir de um texto, com a voz pt-BR do Windows.
# Parágrafos separados por linha em branco viram falas com uma pausa entre elas.
#
#   .\scripts\gerar-audio-teste.ps1 testes\resposta-pagamentos.txt
param(
    [Parameter(Mandatory = $true)][string]$Texto,
    [double]$PausaSegundos = 1.5,
    [string]$Voz = "Microsoft Maria Desktop"
)

Add-Type -AssemblyName System.Speech
$entrada = Resolve-Path $Texto
$saida = [System.IO.Path]::ChangeExtension($entrada, ".wav")
$paragrafos = (Get-Content $entrada -Raw -Encoding UTF8) -split "(\r?\n){2,}" | Where-Object { $_.Trim() }

$sintetizador = New-Object System.Speech.Synthesis.SpeechSynthesizer
$sintetizador.SelectVoice($Voz)
$formato = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$sintetizador.SetOutputToWaveFile($saida, $formato)

$prompt = New-Object System.Speech.Synthesis.PromptBuilder
foreach ($p in $paragrafos) {
    $prompt.AppendText($p.Trim())
    $prompt.AppendBreak([TimeSpan]::FromSeconds($PausaSegundos))
}
$sintetizador.Speak($prompt)
$sintetizador.Dispose()
Write-Output "Gerado: $saida"
