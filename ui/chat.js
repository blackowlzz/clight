import * as ed from './editor.js'
import { $, h, invoke, listen, app, state, toast, guard } from './core.js'
import { endpoint, ready } from './ai.js'

const SYSTEM = 'You are a helpful assistant for C and C++ programmers, inside the clight editor. Be brief and concrete. Put code in fenced code blocks.'

const view = $('v-chat')
const log = h('div', { class: 'chat-log' })
const input = h('textarea', { class: 'chat-input', rows: '2', spellcheck: 'false', placeholder: 'Ask about your code…  Enter sends, Shift+Enter adds a line' })
const sendBtn = h('button', { class: 'primary' }, 'Send')
const withFile = h('input', { type: 'checkbox', checked: true })
view.replaceChildren(log, h('div', { class: 'chat-composer' }, input, h('div', { class: 'chat-bar' }, h('label', { class: 'chat-opt' }, withFile, 'Include the open file'), h('span', { class: 'grow' }), sendBtn)))

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

function welcome() {
  log.replaceChildren()
  const connected = ready()
  const box = h('div', { class: 'chat-welcome' },
    h('b', {}, connected ? 'Ask about your code' : 'Connect a model to start'),
    h('span', { class: 'muted' }, connected ? `Using ${endpoint().model}. Tick “Include the open file” to give it your code.` : 'Add the address of Ollama, LM Studio, llama.cpp or an online service.'),
    connected ? null : h('button', { class: 'primary', onclick: () => app.openSettings?.('ai') }, 'Open AI settings'),
  )
  log.append(box)
}

function setBusy(on) {
  busy = on
  sendBtn.textContent = on ? 'Stop' : 'Send'
  sendBtn.classList.toggle('primary', !on)
}

let frame = 0
function paint() {
  if (frame || !current) return
  frame = requestAnimationFrame(() => {
    frame = 0
    if (current) follow(() => current.body.replaceChildren(render(current.text)))
  })
}

listen('ai-chunk', (e) => {
  if (!current || e.payload.id !== current.id) return
  current.text += e.payload.text
  paint()
})

async function ask() {
  const text = input.value.trim()
  if (!text) return
  if (!ready()) {
    toast('Connect a model first, in Settings > AI')
    return app.openSettings?.('ai')
  }
  if (!history.length) log.replaceChildren()
  input.value = ''
  addMessage('user', text)
  history.push({ role: 'user', content: text })
  const messages = [{ role: 'system', content: SYSTEM }]
  if (withFile.checked && state.active !== null) {
    const ctx = await invoke('doc_context', { id: state.active, max: 12000 }).catch(() => null)
    if (ctx) messages.push({ role: 'system', content: `The user has "${ctx[0]}" open:\n\`\`\`\n${ctx[1]}\n\`\`\`` })
  }
  messages.push(...history.slice(-12))
  const id = ++reqId
  current = { id, text: '', body: addMessage('assistant') }
  setBusy(true)
  try {
    await invoke('ai_chat', { id, endpoint: endpoint(), messages })
  } catch (e) {
    follow(() => current.body.append(h('div', { class: 'chat-error' }, String(e))))
  }
  if (frame) cancelAnimationFrame(frame), (frame = 0)
  follow(() => current.body.replaceChildren(render(current.text), ...[...current.body.querySelectorAll('.chat-error')]))
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

export function shown() {
  status.textContent = ready() ? endpoint().model : 'not connected'
  if (!history.length) welcome()
  input.focus({ preventScroll: true })
}
