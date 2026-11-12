// The hub's asynchronous snapshots can change its height after Vue mounts it.
// Restore its scroll position only after the initial content has rendered.
let layoutReady: Promise<void> = Promise.resolve()

export function beginSystemLayout(): () => void {
  let finish!: () => void
  layoutReady = new Promise<void>(resolve => { finish = resolve })
  return finish
}

export function waitForSystemLayout(): Promise<void> {
  return layoutReady
}
