const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 365 * 24 * 60 * 60 * 1000],
  ['month', 30 * 24 * 60 * 60 * 1000],
  ['day', 24 * 60 * 60 * 1000],
  ['hour', 60 * 60 * 1000],
  ['minute', 60 * 1000],
  ['second', 1000],
]

export function formatRelativeTime(value: string, locale = 'en', now = Date.now()): string {
  const timestamp = new Date(value).getTime()
  if (Number.isNaN(timestamp)) return '—'

  const delta = timestamp - now
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' })

  for (const [unit, ms] of UNITS) {
    if (Math.abs(delta) < ms && unit !== 'second') continue
    const amount = unit === 'second' ? Math.round(delta / ms) : Math.trunc(delta / ms)
    return formatter.format(amount, unit)
  }

  return '—'
}
