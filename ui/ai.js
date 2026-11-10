import { h, state, settings, load, store, app, invoke, listen, toast } from './core.js'
import { notify, closeNote } from './notify.js'

const OFFER = 'ai-dismissed'
const REMIND_MS = 14 * 24 * 3600 * 1000

export const PRESETS = [
  ['ollama', 'Ollama', 'http://127.0.0.1:11434/v1'],
  ['lmstudio', 'LM Studio', 'http://127.0.0.1:1234/v1'],
  ['llama', 'llama.cpp', 'http://127.0.0.1:8080/v1'],
]

// Rough figures for a 4-bit model: the file plus the memory the conversation needs.
export const MODELS = {
  small: { label: 'Small', params: '0.5B', tool: 'ai-model', download: 'about 420 MB', ram: 'about 0.6 GB', cpu: 'light', answers: 'quick and simple, it slips more often', tag: 'Recommended for weak laptops' },
  large: { label: 'Better', params: '1.5B', tool: 'ai-model-large', download: 'about 1 GB', ram: 'about 1.3 GB', cpu: 'heavier, about 3 times slower', answers: 'smarter, but still not perfect', tag: 'For laptops with 8 GB of RAM or more' },
}
export const size = () => (settings.aiSize === 'large' ? 'large' : 'small')
const changed = () => window.dispatchEvent(new Event('ai-changed'))

const installedSizes = { small: false, large: false }
let busy = false

export const isBuiltin = () => settings.aiProvider === 'builtin'
export const installing = () => busy
export const ready = () => (isBuiltin() ? installedSizes[size()] : !!settings.aiUrl.trim() && !!settings.aiModel.trim())
export const endpoint = () => (isBuiltin() ? { url: 'builtin', key: '', model: size() } : { url: settings.aiUrl.trim(), key: settings.aiKey.trim(), model: settings.aiModel.trim() })

export async function refresh() {
  for (const k of Object.keys(installedSizes)) installedSizes[k] = await invoke('ai_builtin_installed', { size: k }).catch(() => false)
  return installedSizes[size()]
}
export const anyInstalled = () => installedSizes.small || installedSizes.large
export const isInstalled = (k) => installedSizes[k]

/** A line about this computer, when it helps to choose. */
export function advice() {
  const { total_mb: ram, cores } = state.sys
  const gb = Math.round(ram / 1024)
  if (ram && ram < 6000) return `This computer has about ${gb} GB of RAM, so the small one is the safe choice.`
  if (ram && ram >= 8000 && cores >= 4) return `This computer has about ${gb} GB of RAM and ${cores} cores, so either one will run.`
  return ''
}

/** Two cards to pick the model from, with what each one costs. */
export function modelCards(onPick) {
  const card = (k) => {
    const m = MODELS[k]
    return h('div', { class: 'ai-card' },
      h('b', {}, `${m.label} · ${m.params}`),
      h('span', { class: 'ai-tag' }, m.tag),
      h('ul', {}, h('li', {}, `Download: ${m.download}`), h('li', {}, `Memory: ${m.ram} while it runs`), h('li', {}, `Processor: ${m.cpu}`), h('li', {}, `Answers: ${m.answers}`)),
      h('button', { class: k === 'small' ? 'primary' : '', onclick: () => onPick(k) }, `Install ${m.label.toLowerCase()}`),
    )
  }
  return h('div', { class: 'ai-cards' }, card('small'), card('large'), h('p', { class: 'muted ai-fine' }, `These are estimates. The AI stops by itself after 5 idle minutes, so it uses no memory while you are not chatting. ${advice()}`.trim()))
}

function save(key, v) {
  settings[key] = v
  store(key, v)
}

export async function installBuiltin(pick = size()) {
  if (busy) return
  busy = true
  changed()
  const wanted = ['ai-engine', MODELS[pick].tool]
  const todo = (await invoke('tools_status')).filter((t) => wanted.includes(t.id) && !t.installed)
  const total = todo.reduce((n, t) => n + t.size, 0) || 1
  const got = {}
  const show = (progress, body) => notify({ id: 'ai-install', title: `Installing the ${MODELS[pick].label.toLowerCase()} AI`, body, progress })
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
    save('aiSize', pick)
    save('aiChat', true)
    await refresh()
    app.syncChat?.()
    closeNote('ai')
    notify({
      id: 'ai-ready',
      title: 'AI ready',
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

export async function removeBuiltin(pick = size()) {
  await invoke('ai_remove', { size: pick })
  await refresh()
  changed()
}

function offer() {
  const dismiss = () => store(OFFER, Date.now())
  const tip = advice()
  notify({
    id: 'ai',
    title: 'AI assistant',
    body: `A chat for your code that runs on your computer, with nothing to set up. Small: ${MODELS.small.download}, ${MODELS.small.ram} of memory, light on the processor. Better: ${MODELS.large.download}, ${MODELS.large.ram} of memory, about 3 times slower but smarter. Both can make mistakes, and they stop by themselves when idle. ${tip}`.trim(),
    actions: [
      { label: `Small · 0.5B (${MODELS.small.download})`, primary: true, run: () => installBuiltin('small') },
      { label: `Better · 1.5B (${MODELS.large.download})`, run: () => installBuiltin('large') },
      { label: 'Not now', run: dismiss },
    ],
    onDismiss: dismiss,
  })
}

export async function startup() {
  await refresh()
  if (settings.aiChat || anyInstalled() || Date.now() - load(OFFER, 0) < REMIND_MS) return
  setTimeout(offer, 9000)
}
