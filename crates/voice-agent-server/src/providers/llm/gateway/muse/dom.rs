//! DOM selectors and in-page scripts for the `muse.ai` Hatch web client.

pub const INPUT_SELECTOR: &str = "textarea";
pub const FILE_INPUT_SELECTOR: &str = r#"input[type="file"]"#;

pub const LOGIN_HINTS: &[&str] = &[
    "log in",
    "sign in",
    "create an account",
    "use another account",
];

pub const QUOTA_HINTS: &[&str] = &[
    "out of credits",
    "usage limit",
    "limit reached",
    "token limit",
];

pub const STALL_HINTS: &[&str] = &["Still sending", "Connecting..."];

pub const GLITCH_HINTS: &[&str] = &["ran into a problem while responding"];

pub fn is_glitch_reply(text: &str) -> bool {
    if text.len() >= 300 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    GLITCH_HINTS.iter().any(|hint| lower.contains(hint))
}

fn json_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

pub const PAGE_STATE_JS: &str = r#"(() => {
  const body = document.body ? document.body.innerText : '';
  const hydration = document.querySelector('[data-hatch-shell-hydration-state]');
  return {
    ready: document.readyState === 'complete' && !!document.querySelector("textarea")
           && (!hydration || hydration.getAttribute('data-hatch-shell-hydration-state') === 'hydrated')
           && !body.includes('Connecting...'),
    hasInput: !!document.querySelector("textarea"),
    path: location.pathname,
    head: body.slice(0, 800),
  };
})()"#;

pub const CHAT_STATE_JS: &str = r#"(() => {
  const scrollers = [...document.querySelectorAll('*')].filter(e => {
    const s = getComputedStyle(e);
    return (s.overflowY === 'auto' || s.overflowY === 'scroll') && e.scrollHeight > e.clientHeight + 50;
  });
  scrollers.forEach(e => { e.scrollTop = e.scrollHeight; });
  const bubbles = [...document.querySelectorAll("div[class*=\"hatch-agent-bubble-bg\"]")];
  const bubbleTexts = bubbles.map(b => (b.innerText || '').trim());
  const last = bubbleTexts.length ? bubbleTexts[bubbleTexts.length - 1] : '';
  const atts = [...document.querySelectorAll("[data-testid^=\"hatch-chat-attachment-presentation-\"]")].flatMap((a, group) => {
    const tid = a.getAttribute('data-testid') || '';
    const skeletons = a.querySelectorAll('[data-slot="skeleton"], [data-visualcompletion="loading-state"]').length;
    const vids = [...a.querySelectorAll('video')];
    const isVideo = tid.includes('video') || vids.length > 0 || !!a.querySelector('[data-hatch-video-wrapper]');
    const loadingKind = isVideo ? 'video' : (tid.includes('image') ? 'image' : 'unknown');
    const items = isVideo
      ? vids.map((v, idx) => {
          const wrap = v.closest('[data-hatch-video-src]');
          return {
            tid: vids.length > 1 ? `${tid}-${idx}` : tid,
            group,
            kind: 'video',
            src: v.currentSrc || v.src || (wrap ? wrap.getAttribute('data-hatch-video-src') || '' : ''),
            w: v.videoWidth || 0,
            h: v.videoHeight || 0,
          };
        })
      : [...a.querySelectorAll('img:not([aria-hidden="true"])')].map((i, idx, arr) => ({
          tid: arr.length > 1 ? `${tid}-${idx}` : tid,
          group,
          kind: 'image',
          src: i.currentSrc || i.src || '',
          w: i.naturalWidth || 0,
          h: i.naturalHeight || 0,
        }));
    for (let k = 0; k < skeletons; k++) {
      items.push({ tid: `${tid}-loading-${k}`, group, kind: loadingKind, src: '', w: 0, h: 0 });
    }
    if (items.length === 0) {
      items.push({ tid, group, kind: 'unknown', src: '', w: 0, h: 0 });
    }
    return items;
  });
  const tail = document.body ? document.body.innerText.slice(-600) : '';
  return {
    agentCount: bubbles.length,
    lastText: last,
    bubbles: bubbleTexts,
    generating: !!document.querySelector(
      "button[aria-label*=\"Stop\" i], [data-testid=\"hatch-composer-stop-button\"], [data-testid=\"hatch-chat-typing-indicator\"]"
    ),
    approval: !!document.querySelector("[data-testid=\"hatch-inline-approval-card\"]"),
    attachments: atts,
    tail
  };
})()"#;

pub const DENY_APPROVALS_JS: &str = r#"(() => {
  const denied = [];
  for (const card of document.querySelectorAll("[data-testid=\"hatch-inline-approval-card\"]")) {
    const btn = [...card.querySelectorAll('button,[role=button]')]
      .find(b => /^(deny|decline|reject|don.t allow)$/i.test((b.innerText || '').trim()));
    if (btn && !btn.disabled) {
      btn.click();
      denied.push(card.getAttribute('aria-label') || 'approval request');
    }
  }
  return denied;
})()"#;

pub const LAST_BUBBLE_MEDIA_JS: &str = r#"(() => {
  const bubbles = [...document.querySelectorAll("div[class*=\"hatch-agent-bubble-bg\"]")];
  const b = bubbles.length ? bubbles[bubbles.length - 1] : null;
  if (!b) return { links: [], html: '' };
  const urls = [];
  const push = (u) => { if (u && !urls.includes(u)) urls.push(u); };
  document.querySelectorAll("[data-testid^=\"hatch-chat-attachment-presentation-\"] video, [data-testid^=\"hatch-chat-attachment-presentation-\"] source").forEach(m => push(m.currentSrc || m.src || ''));
  b.querySelectorAll('video, source').forEach(m => push(m.currentSrc || m.src || ''));
  b.querySelectorAll('a[href]').forEach(a => push(a.href || ''));
  b.querySelectorAll('img:not([aria-hidden="true"])').forEach(m => push(m.currentSrc || m.src || ''));
  const text = b.innerText || '';
  const re = /https?:\/\/[^\s)<>"']+/g;
  let mm; while ((mm = re.exec(text))) push(mm[0]);
  return { links: urls, html: (b.outerHTML || '').slice(0, 2000) };
})()"#;

pub const CLICK_SEND_JS: &str = r#"(() => {
  const btn = [...document.querySelectorAll('button,[role=button]')]
    .filter(b => b.offsetParent !== null)
    .find(b => /send|发送/i.test((b.getAttribute('aria-label') || '') + ' ' + (b.getAttribute('data-testid') || '')));
  if (!btn) return 'missing';
  if (btn.disabled) return 'disabled';
  btn.click();
  return 'clicked';
})()"#;

pub const INPUT_EMPTY_JS: &str =
    r#"(() => { const el = document.querySelector("textarea"); return !el || !el.value.trim(); })()"#;

pub fn fill_input_js(text: &str) -> String {
    let q_input = json_quote(INPUT_SELECTOR);
    let q_text = json_quote(text);
    format!(
        r#"(() => {{
  const ta = document.querySelector({q_input});
  if (!ta) return {{ ok: false, err: 'no-input' }};
  ta.focus();
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
  setter.call(ta, {q_text});
  ta.dispatchEvent(new Event('input', {{ bubbles: true }}));
  return {{ ok: true, len: ta.value.length }};
}})()"#
    )
}

pub fn attach_file_js(b64: &str, mime: &str, filename: &str) -> String {
    let q_file = json_quote(FILE_INPUT_SELECTOR);
    let q_b64 = json_quote(b64);
    let q_mime = json_quote(mime);
    let q_name = json_quote(filename);
    format!(
        r#"(() => {{
  try {{
    const input = document.querySelector({q_file});
    if (!input) return {{ ok: false, err: 'no-file-input' }};
    const bin = atob({q_b64});
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const dt = new DataTransfer();
    dt.items.add(new File([bytes], {q_name}, {{ type: {q_mime} }}));
    input.files = dt.files;
    input.dispatchEvent(new Event('change', {{ bubbles: true }}));
    return {{ ok: true }};
  }} catch (e) {{ return {{ ok: false, err: String(e) }}; }}
}})()"#
    )
}

pub fn fetch_as_base64_js(url: &str) -> String {
    let q_url = json_quote(url);
    format!(
        r#"(async () => {{
  try {{
    const r = await fetch({q_url});
    const b = await r.blob();
    const buf = new Uint8Array(await b.arrayBuffer());
    let s = '';
    for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
    return {{ ok: true, mime: b.type || '', b64: btoa(s) }};
  }} catch (e) {{ return {{ ok: false, err: String(e) }}; }}
}})()"#
    )
}

pub fn body_includes_js(snippet: &str) -> String {
    let q_snippet = json_quote(snippet);
    format!(r#"!!(document.body && document.body.innerText.includes({q_snippet}))"#)
}
