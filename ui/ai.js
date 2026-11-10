import { settings, load, store, app } from './core.js'
import { notify } from './notify.js'

const OFFER = 'ai-dismissed'
const REMIND_MS = 14 * 24 * 3600 * 1000

export const PRESETS = [
  ['ollama', 'Ollama', 'http://127.0.0.1:11434/v1'],
  ['lmstudio', 'LM Studio', 'http://127.0.0.1:1234/v1'],
  ['llama', 'llama.cpp', 'http://127.0.0.1:8080/v1'],
]

export const endpoint = () => ({ url: settings.aiUrl.trim(), key: settings.aiKey.trim(), model: settings.aiModel.trim() })
export const ready = () => !!settings.aiUrl.trim() && !!settings.aiModel.trim()

function offer() {
  const dismiss = () => store(OFFER, Date.now())
  notify({
    id: 'ai',
    title: 'AI assistant',
    body: 'Add a chat like the one in VS Code and connect your own model: Ollama, LM Studio, llama.cpp or an online service. Nothing runs unless you turn it on.',
    actions: [
      { label: 'Set up AI', primary: true, run: () => app.openSettings?.('ai') },
      { label: 'Not now', run: dismiss },
    ],
    onDismiss: dismiss,
  })
}

export function startup() {
  if (settings.aiChat || Date.now() - load(OFFER, 0) < REMIND_MS) return
  setTimeout(offer, 9000)
}
