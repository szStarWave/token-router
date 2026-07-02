export async function openExternalUrl(url: string): Promise<void> {
  const tauri = window as Window & { __TAURI__?: { opener?: { openUrl: (u: string) => Promise<void> } } }
  if (tauri.__TAURI__?.opener?.openUrl) {
    await tauri.__TAURI__.opener.openUrl(url)
    return
  }
  window.open(url, '_blank', 'noopener,noreferrer')
}
