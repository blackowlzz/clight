import { h } from './core.js'

const stack = h('div', { id: 'notes' })
document.body.append(stack)
const cards = new Map()

function make(id) {
  const title = h('div', { class: 'note-title' })
  const body = h('div', { class: 'note-body' })
  const fill = h('i')
  const bar = h('div', { class: 'note-bar' }, fill)
  const actions = h('div', { class: 'note-actions' })
  const card = { id, onDismiss: null }
  const x = h('button', { class: 'note-x', title: 'Dismiss', 'aria-label': 'Dismiss', onclick: () => (card.onDismiss?.(), card.close()) }, '×')
  card.el = h('div', { class: 'note', role: 'status' }, h('div', {}, title, body, bar, actions), x)
  card.set = ({ title: t, body: b = '', actions: a = [], progress = null }) => {
    title.textContent = t
    body.textContent = b
    body.hidden = !b
    bar.hidden = progress === null
    fill.style.width = Math.round((progress ?? 0) * 100) + '%'
    actions.hidden = !a.length
    actions.replaceChildren(
      ...a.map((item) =>
        h(
          'button',
          {
            class: item.primary ? 'primary' : '',
            onclick: () => {
              item.run?.()
              if (!item.keep) card.close()
            },
          },
          item.label,
        ),
      ),
    )
  }
  card.close = () => {
    cards.delete(id)
    if (document.documentElement.dataset.fxUi !== 'on') return card.el.remove()
    card.el.style.pointerEvents = 'none'
    card.el.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateX(14px)' }], { duration: 160, easing: 'ease-in' }).onfinish = () => card.el.remove()
  }
  return card
}

/** A card in the bottom right corner. Calling it again with the same id updates that card. */
export function notify({ id, title, body, actions, progress, onDismiss }) {
  let card = cards.get(id)
  if (!card) {
    card = make(id)
    cards.set(id, card)
    stack.append(card.el)
  }
  card.onDismiss = onDismiss ?? null
  card.set({ title, body, actions, progress })
  return card
}

export const closeNote = (id) => cards.get(id)?.close()
