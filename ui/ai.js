import { settings, load, store, app, invoke, listen, toast } from './core.js'
import { notify, closeNote } from './notify.js'

const OFFER = 'ai-dismissed'
const REMIND_MS = 14 * 24 * 3600 * 1000

export const PRESETS = [
  ['ollama', 'Ollama', 'http://127.0.0.1:11434/v1'],
  ['lmstudio', 'LM Studio', 'http://127.0.0.1:1234/v1'],
  ['llama', 'llama.cpp', 'http://127.0.0.1:8080/v1'],
]

export const SMALL_AI_SIZE = '420 MB'
const changed = () => window.dispatchEvent(new Event('ai-changed'))

let installed = false
let busy = false

export const isBuiltin = () => settings.aiProvider === 'builtin'
export const installing = () => busy
export const ready = () => (isBuiltin() ? installed : !!settings.aiUrl.trim() && !!settings.aiModel.trim())
export const endpoint = () => (isBuiltin() ? { url: 'builtin', key: '', model: '' } : { url: settings.aiUrl.trim(), key: settings.aiKey.trim(), model: settings.aiModel.trim() })

export async function refresh() {
  installed = await invoke('ai_builtin_installed').catch(() => false)
  return installed
}

function save(key, v) {
  settings[key] = v
  store(key, v)
}

export async function installBuiltin() {
  if (busy) return
  busy = true
  changed()
  const todo = (await invoke('tools_status')).filter((t) => t.group === 'ai' && !t.installed)
  const total = todo.reduce((n, t) => n + t.size, 0) || 1
  const got = {}
  const show = (progress, body) => notify({ id: 'ai-install', title: 'Installing the small AI', body, progress })
  show(0, 'Starting…')
  const off = await listen('tools-progress', (e) => {
    const { id, done } = e.payload
    const tool = todo.find((t) => t.id === id)
    if (!tool) return
    got[id] = done
    const sum = Object.values(got).reduce((n, v) => n + v, 0)
    show(Math.min(1, sum / total), `${tool.title}  ${Math.min(100, Math.floor((done / tool.size) * 100))}%`)
  })
  try {
    const failures = await invoke('tools_install', { ids: todo.map((t) => t.id) })
    if (failures.length) throw new Error(`${failures[0].title}: ${failures[0].error}`)
    save('aiProvider', 'builtin')
    save('aiChat', true)
    await refresh()
    app.syncChat?.()
    closeNote('ai')
    notify({
      id: 'ai-ready',
      title: 'Small AI ready',
      body: 'It starts by itself when you use the chat. It is a small model, so it can make mistakes.',
      actions: [{ label: 'Open the chat', primary: true, run: () => app.toggleChat?.(true) }, { label: 'Close' }],
    })
  } catch (e) {
    toast(String(e.message ?? e))
  } finally {
    off()
    closeNote('ai-install')
    busy = false
    changed()
  }
}

export async function removeBuiltin() {
  await invoke('ai_remove')
  await refresh()
  changed()
}

function offer() {
  const dismiss = () => store(OFFER, Date.now())
  notify({
    id: 'ai',
    title: 'AI assistant',
    body: `A small AI for your code, in a chat on the right. One click installs it (about ${SMALL_AI_SIZE}) and it runs on your computer, with nothing to set up. It is a small model, so it can make mistakes.`,
    actions: [
      { label: 'Install the small AI', primary: true, run: () => installBuiltin() },
      { label: 'Advanced setup', run: () => app.openSettings?.('ai') },
      { label: 'Not now', run: dismiss },
    ],
    onDismiss: dismiss,
  })
}

export async function startup() {
  await refresh()
  if (settings.aiChat || installed || Date.now() - load(OFFER, 0) < REMIND_MS) return
  setTimeout(offer, 9000)
}
