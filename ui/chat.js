import * as ed from './editor.js'
import { $, h, icon, invoke, listen, app, state, toast, guard } from './core.js'
import * as ai from './ai.js'
import { endpoint, ready } from './ai.js'

const SYSTEM = 'You are a helpful assistant for C and C++ programmers, inside the clight editor. Be brief and concrete. Put code in fenced code blocks.'

const view = $('v-chat')
const log = h('div', { class: 'chat-log' })
const input = h('textarea', { class: 'chat-input', rows: '1', spellcheck: 'false', placeholder: 'Ask about your code…' })
const sendBtn = h('button', { class: 'chat-send', title: 'Send (Enter)', 'aria-label': 'Send', disabled: true }, icon('send'))
const note = h('div', { class: 'chat-note' })
let fileOn = true
const fileChip = h('button', {
  class: 'chat-chip on',
  type: 'button',
  'aria-pressed': 'true',
  title: 'Send the file you have open along with your question',
  onclick: () => {
    fileOn = !fileOn
    fileChip.classList.toggle('on', fileOn)
    fileChip.setAttribute('aria-pressed', String(fileOn))
  },
}, 'Open file')
/** The chip names the file that goes with the question: the one of the editor you were last in,
 * which matters once the editor is split in two. */
export function syncFile() {
  const t = state.tabs.find((x) => x.id === state.active)
  fileChip.textContent = t ? t.name : 'No file open'
  fileChip.disabled = !t
  fileChip.title = t ? `Send ${t.name} along with your question (the file of the editor you were last in)` : 'Open a file to send it along'
}
syncFile()
const box = h('div', { class: 'chat-box' }, input, h('div', { class: 'chat-foot' }, fileChip, h('span', { class: 'grow' }), h('span', { class: 'chat-hint' }, 'Enter to send'), sendBtn))
view.replaceChildren(log, h('div', { class: 'chat-composer' }, box, note))

function fit() {
  input.style.height = 'auto'
  input.style.height = Math.min(input.scrollHeight, 140) + 'px'
  sendBtn.disabled = !busy && !input.value.trim()
}
input.addEventListener('input', fit)

let history = []
let busy = false
let reqId = 0
let current = null

function inline(text) {
  return text.split(/(`[^`\n]+`|\*\*[^*\n]+\*\*)/).map((p) => {
    if (p.length > 2 && p.startsWith('`')) return h('code', {}, p.slice(1, -1))
    if (p.length > 4 && p.startsWith('**')) return h('b', {}, p.slice(2, -2))
    return p
  })
}

function codeBlock(part) {
  const nl = part.indexOf('\n')
  const lang = nl < 0 ? '' : part.slice(0, nl).trim()
  const code = (nl < 0 ? part : part.slice(nl + 1)).replace(/\n$/, '')
  const copy = h('button', { class: 'plain', onclick: () => invoke('clipboard_write', { text: code }).then(() => toast('Copied')).catch(toast) }, 'Copy')
  const insert = h('button', {
    class: 'plain',
    onclick: () => {
      if (state.active === null) return toast('Open a file to insert the code')
      ed.send({ t: 'insert', s: code })
      ed.focus()
    },
  }, 'Insert')
  return h('div', { class: 'chat-code' }, h('div', { class: 'chat-codehead' }, h('span', {}, lang || 'code'), h('span', { class: 'grow' }), copy, insert), h('pre', {}, code))
}

function render(text) {
  const frag = document.createDocumentFragment()
  text.split('```').forEach((part, i) => {
    if (i % 2) frag.append(codeBlock(part))
    else if (part.trim()) frag.append(h('div', { class: 'chat-text' }, ...inline(part.trim())))
  })
  return frag
}

function follow(fn) {
  const near = log.scrollTop + log.clientHeight >= log.scrollHeight - 40
  fn()
  if (near) log.scrollTop = log.scrollHeight
}

function addMessage(role, text = '') {
  const body = h('div', { class: 'chat-body' })
  const row = h('div', { class: 'chat-msg ' + role }, h('div', { class: 'chat-who' }, role === 'user' ? 'You' : 'AI'), body)
  follow(() => {
    log.append(row)
    if (text) body.replaceChildren(...(role === 'user' ? [h('div', { class: 'chat-text' }, text)] : [render(text)]))
  })
  return body
}

const STARTERS = [
  ['Explain this file', 'Explain what the open file does, briefly.'],
  ['Find bugs', 'Look for bugs in the open file and list them.'],
  ['Suggest improvements', 'Suggest improvements for the open file.'],
]

function updateNote() {
  note.textContent = ai.isBuiltin() && ready() ? `${ai.MODELS[ai.size()].label} AI (${ai.MODELS[ai.size()].params}): it can make mistakes, so check what it writes.` : ''
  note.hidden = !note.textContent
}

function welcome() {
  log.replaceChildren()
  updateNote()
  let box
  if (ai.isBuiltin() && !ready()) {
    box = h('div', { class: 'chat-welcome' },
      h('b', {}, 'An AI for your code, on your computer'),
      h('span', { class: 'muted' }, 'Pick one. It installs and starts by itself, with nothing to set up.'),
      ai.installing() ? h('span', { class: 'muted' }, 'Installing… see the notification in the corner.') : ai.modelCards((k) => guard(ai.installBuiltin(k))),
      h('button', { class: 'plain', onclick: () => app.openSettings?.('ai') }, 'Use my own server instead'),
    )
  } else if (ready()) {
    box = h('div', { class: 'chat-welcome' },
      h('b', {}, 'Ask about your code'),
      h('span', { class: 'muted' }, (ai.isBuiltin() ? `Using the ${ai.MODELS[ai.size()].label.toLowerCase()} built-in AI (${ai.MODELS[ai.size()].params}). It is a small model and can make mistakes.` : `Using ${endpoint().model}.`) + ' It sees the file you have open.'),
      state.active === null ? null : h('div', { class: 'chat-starters' }, ...STARTERS.map(([label, prompt]) => h('button', { class: 'chat-starter', onclick: () => { input.value = prompt; fit(); guard(ask()) } }, label))),
    )
  } else {
    box = h('div', { class: 'chat-welcome' },
      h('b', {}, 'Connect a model to start'),
      h('span', { class: 'muted' }, 'Add the address of Ollama, LM Studio, llama.cpp or an online service.'),
      h('button', { class: 'primary', onclick: () => app.openSettings?.('ai') }, 'Open AI settings'),
    )
  }
  log.append(box)
}

function setBusy(on) {
  busy = on
  sendBtn.querySelector('use').setAttribute('href', on ? '#i-stop' : '#i-send')
  sendBtn.title = on ? 'Stop' : 'Send (Enter)'
  sendBtn.setAttribute('aria-label', sendBtn.title)
  box.classList.toggle('busy', on)
  fit()
}

const FRESH_MS = 320
const fx = () => document.documentElement.dataset.fxUi === 'on'

function dots() {
  return h('span', { class: 'chat-thinking' }, h('i'), h('i'), h('i'), h('span', { class: 'chat-status' }))
}

/** Wraps the text that arrived in the last moments, so it fades in and keeps going across redraws. */
function freshen(body, run) {
  const nodes = []
  const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => (n.parentElement.closest('.chat-codehead, .chat-cursor') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  })
  let total = ''
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    nodes.push([n, total.length])
    total += n.data
  }
  const now = performance.now()
  let keep = 0
  const limit = Math.min(run.seen.length, total.length)
  while (keep < limit && run.seen[keep] === total[keep]) keep++
  run.marks = run.marks.filter((m) => m.from < keep && now - m.t < FRESH_MS).map((m) => ({ ...m, to: Math.min(m.to, keep) }))
  if (total.length > keep) run.marks.push({ from: keep, to: total.length, t: now })
  run.seen = total
  const jobs = []
  for (const m of run.marks) {
    for (const [node, start] of nodes) {
      const a = Math.max(m.from, start) - start
      const b = Math.min(m.to, start + node.data.length) - start
      if (b > a) jobs.push({ node, a, b, age: now - m.t })
    }
  }
  jobs.sort((x, y) => (x.node === y.node ? y.a - x.a : 0))
  for (const j of jobs) {
    const span = h('span', { class: 'tok' })
    span.style.animationDelay = `-${Math.round(j.age)}ms`
    const range = document.createRange()
    range.setStart(j.node, j.a)
    range.setEnd(j.node, j.b)
    range.surroundContents(span)
  }
}

function draw(run, live) {
  const parts = [render(run.text)]
  if (live && run.text) parts.push(h('span', { class: 'chat-cursor' }))
  run.body.replaceChildren(...parts)
  if (live && fx()) freshen(run.body, run)
}

let frame = 0
function paint() {
  if (frame || !current) return
  frame = requestAnimationFrame(() => {
    frame = 0
    if (current) follow(() => draw(current, true))
  })
}

listen('ai-status', (e) => {
  if (current && e.payload.id === current.id) {
    const el = current.body.querySelector('.chat-status')
    if (el) el.textContent = e.payload.text
  }
})

window.addEventListener('ai-changed', () => {
  updateNote()
  status.textContent = ready() ? (ai.isBuiltin() ? `${ai.MODELS[ai.size()].label.toLowerCase()} AI` : endpoint().model) : 'not connected'
  if (!history.length && !current) welcome()
})

listen('ai-chunk', (e) => {
  if (!current || e.payload.id !== current.id) return
  current.text += e.payload.text
  paint()
})

async function ask() {
  const text = input.value.trim()
  if (!text) return
  if (!ready()) {
    welcome()
    return toast(ai.isBuiltin() ? 'Install the AI first' : 'Connect a model first, in Settings > AI')
  }
  if (!history.length) log.replaceChildren()
  input.value = ''
  fit()
  addMessage('user', text)
  history.push({ role: 'user', content: text })
  const messages = [{ role: 'system', content: SYSTEM }]
  if (fileOn && state.active !== null) {
    const ctx = await invoke('doc_context', { id: state.active, max: 12000 }).catch(() => null)
    if (ctx) messages.push({ role: 'system', content: `The user has "${ctx[0]}" open:\n\`\`\`\n${ctx[1]}\n\`\`\`` })
  }
  messages.push(...history.slice(-12))
  const id = ++reqId
  current = { id, text: '', body: addMessage('assistant'), seen: '', marks: [] }
  current.body.append(dots())
  setBusy(true)
  try {
    await invoke('ai_chat', { id, endpoint: endpoint(), messages })
  } catch (e) {
    follow(() => current.body.append(h('div', { class: 'chat-error' }, String(e))))
  }
  if (frame) cancelAnimationFrame(frame), (frame = 0)
  const errors = [...current.body.querySelectorAll('.chat-error')]
  follow(() => {
    if (current.text) draw(current, false)
    else current.body.replaceChildren()
    current.body.append(...errors)
  })
  if (current.text) history.push({ role: 'assistant', content: current.text })
  current = null
  setBusy(false)
}

sendBtn.addEventListener('click', () => (busy ? invoke('ai_cancel', { id: reqId }) : guard(ask())))
input.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault()
    if (!busy) guard(ask())
  }
})

const status = h('span', { class: 'muted' })
const panel = h('span', { class: 'chat-actions' }, status, h('button', { class: 'plain', onclick: () => newChat() }, 'New chat'))
export const panelActions = () => panel

function newChat() {
  if (busy) invoke('ai_cancel', { id: reqId })
  history = []
  welcome()
}

export async function shown() {
  await ai.refresh()
  status.textContent = ready() ? (ai.isBuiltin() ? `${ai.MODELS[ai.size()].label.toLowerCase()} AI` : endpoint().model) : 'not connected'
  if (!history.length && !current) welcome()
  input.focus({ preventScroll: true })
  if (ai.isBuiltin() && ready()) invoke('ai_warm', { size: ai.size() }).catch(() => {})
}
