// Voz do entrevistador simulado no treino: a síntese de fala do próprio Windows (pela
// WebView), sem mandar o texto pra mais ninguém.

let chosen: SpeechSynthesisVoice | null = null;

/** As vozes carregam depois da página; na primeira chamada podem ainda não existir. */
function voices(): Promise<SpeechSynthesisVoice[]> {
  const list = speechSynthesis.getVoices();
  if (list.length) return Promise.resolve(list);
  return new Promise((resolve) => {
    const done = () => resolve(speechSynthesis.getVoices());
    speechSynthesis.addEventListener("voiceschanged", done, { once: true });
    setTimeout(done, 1500);
  });
}

/** Uma voz em português do Brasil, de preferência as "naturais". */
async function ptVoice() {
  if (chosen) return chosen;
  const pt = (await voices()).filter((v) => v.lang.toLowerCase().replace("_", "-").startsWith("pt-br"));
  chosen = pt.find((v) => /natural|online/i.test(v.name)) ?? pt[0] ?? null;
  return chosen;
}

/** Fala o texto e resolve com quanto tempo levou, em ms. */
export async function speak(text: string): Promise<number> {
  if (!("speechSynthesis" in window)) return 0;
  speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.lang = "pt-BR";
  utterance.voice = await ptVoice();
  utterance.rate = 1.05;
  const started = performance.now();
  // Se a voz não avisar que terminou (sem voz instalada, áudio travado), o treino não
  // pode ficar parado com o microfone mudo: desiste depois do dobro do tempo esperado.
  const words = text.split(/\s+/).length;
  const limit = words * 900 + 3_000;
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(Math.round(performance.now() - started));
    };
    const timer = setTimeout(() => {
      speechSynthesis.cancel();
      finish();
    }, limit);
    utterance.onend = finish;
    utterance.onerror = finish;
    speechSynthesis.speak(utterance);
  });
}

export function stopSpeaking() {
  if ("speechSynthesis" in window) speechSynthesis.cancel();
}
