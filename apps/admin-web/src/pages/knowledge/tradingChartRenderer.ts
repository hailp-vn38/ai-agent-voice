/**
 * Hydrate Markdown `trading-chart` fences into interactive OHLCV + annotation charts.
 *
 * Deep module with two external seams for testability:
 * - `fetcher`: Admin HTTP client (`GET /api/admin/knowledge/charts/*`) vs in-memory test fake.
 * - `engineAdapter`: Interactive SVG/Canvas chart engine vs in-memory test spy.
 */
import { requestJson } from '@/api/client'

export type TradingChartTone =
  | 'bullish'
  | 'bearish'
  | 'demand'
  | 'supply'
  | 'structure'
  | 'warning'
  | 'neutral'

export type TradingChartLineStyle = 'solid' | 'dashed' | 'dotted'

export type TradingChartPoint = {
  time: number
  price: number
}

export type TradingChartCandle = {
  time: number
  open: number
  high: number
  low: number
  close: number
  volume?: number
  tag?: string
}

export type TradingChartDrawing =
  | {
      id: string
      type: 'horizontal-line'
      price: number
      label?: string
      tone?: TradingChartTone
      style?: TradingChartLineStyle
    }
  | {
      id: string
      type: 'trend-line'
      points: [TradingChartPoint, TradingChartPoint]
      label?: string
      tone?: TradingChartTone
      style?: TradingChartLineStyle
      width?: number
    }
  | {
      id: string
      type: 'price-zone'
      from: TradingChartPoint
      to: TradingChartPoint
      label?: string
      tone?: TradingChartTone
    }
  | {
      id: string
      type: 'text'
      anchor: TradingChartPoint
      text: string
      tone?: TradingChartTone
    }

export type TradingChartDocument = {
  schemaVersion: 1
  revision: number
  id: string
  title: string
  symbol?: string
  timeframe?: string
  display?: {
    height?: number
    showVolume?: boolean
    fitContent?: boolean
  }
  candles: TradingChartCandle[]
  drawings?: TradingChartDrawing[]
}

export type TonePalette = {
  tone: TradingChartTone
  stroke: string
  fill: string
  text: string
}

export type NormalizedTradingChartDrawing =
  | {
      id: string
      type: 'horizontal-line'
      price: number
      label: string
      style: TradingChartLineStyle
      palette: TonePalette
    }
  | {
      id: string
      type: 'trend-line'
      points: [TradingChartPoint, TradingChartPoint]
      label: string
      style: TradingChartLineStyle
      width: number
      palette: TonePalette
    }
  | {
      id: string
      type: 'price-zone'
      from: TradingChartPoint
      to: TradingChartPoint
      label: string
      palette: TonePalette
    }
  | {
      id: string
      type: 'text'
      anchor: TradingChartPoint
      text: string
      palette: TonePalette
    }

export type NormalizedTradingChartModel = {
  id: string
  title: string
  symbol: string
  timeframe: string
  revision: number
  dark: boolean
  display: {
    height: number
    showVolume: boolean
    fitContent: boolean
  }
  candles: TradingChartCandle[]
  drawings: NormalizedTradingChartDrawing[]
}

export interface ChartEngineInstance {
  fitContent(): void
  destroy(): void
}

export interface ChartEngineAdapter {
  mount(container: HTMLElement, model: NormalizedTradingChartModel): ChartEngineInstance
}

export type TradingChartRendererDeps = {
  fetcher?: (src: string) => Promise<unknown>
  engineAdapter?: ChartEngineAdapter
}

const SAFE_CHART_SRC_RE = /^charts\/([a-zA-Z0-9_-]{1,96}\.chart\.json)$/
const VALID_TONES = new Set<TradingChartTone>([
  'bullish',
  'bearish',
  'demand',
  'supply',
  'structure',
  'warning',
  'neutral',
])
const VALID_STYLES = new Set<TradingChartLineStyle>(['solid', 'dashed', 'dotted'])

const DARK_PALETTES: Record<TradingChartTone, TonePalette> = {
  bullish: { tone: 'bullish', stroke: '#34d399', fill: 'rgba(52, 211, 153, 0.16)', text: '#6ee7b7' },
  bearish: { tone: 'bearish', stroke: '#f87171', fill: 'rgba(248, 113, 113, 0.16)', text: '#fca5a5' },
  demand: { tone: 'demand', stroke: '#2dd4bf', fill: 'rgba(45, 212, 191, 0.18)', text: '#5eead4' },
  supply: { tone: 'supply', stroke: '#fbbf24', fill: 'rgba(251, 191, 36, 0.18)', text: '#fcd34d' },
  structure: { tone: 'structure', stroke: '#60a5fa', fill: 'rgba(96, 165, 250, 0.16)', text: '#93c5fd' },
  warning: { tone: 'warning', stroke: '#f59e0b', fill: 'rgba(245, 158, 11, 0.22)', text: '#fde68a' },
  neutral: { tone: 'neutral', stroke: '#94a3b8', fill: 'rgba(148, 163, 184, 0.16)', text: '#cbd5e1' },
}

const LIGHT_PALETTES: Record<TradingChartTone, TonePalette> = {
  bullish: { tone: 'bullish', stroke: '#059669', fill: 'rgba(5, 150, 105, 0.12)', text: '#065f46' },
  bearish: { tone: 'bearish', stroke: '#dc2626', fill: 'rgba(220, 38, 38, 0.12)', text: '#991b1b' },
  demand: { tone: 'demand', stroke: '#0d9488', fill: 'rgba(13, 148, 136, 0.14)', text: '#115e59' },
  supply: { tone: 'supply', stroke: '#d97706', fill: 'rgba(217, 119, 6, 0.14)', text: '#92400e' },
  structure: { tone: 'structure', stroke: '#2563eb', fill: 'rgba(37, 99, 235, 0.12)', text: '#1e40af' },
  warning: { tone: 'warning', stroke: '#b45309', fill: 'rgba(180, 83, 9, 0.16)', text: '#78350f' },
  neutral: { tone: 'neutral', stroke: '#64748b', fill: 'rgba(100, 116, 139, 0.12)', text: '#334155' },
}

type ActiveWidgetState = {
  instance: ChartEngineInstance
  rawDoc: unknown
  dark: boolean
}

const activeWidgets = new WeakMap<HTMLElement, ActiveWidgetState>()

function resolveTonePalette(tone: unknown, dark: boolean): TonePalette {
  const normalized: TradingChartTone =
    typeof tone === 'string' && VALID_TONES.has(tone as TradingChartTone)
      ? (tone as TradingChartTone)
      : 'neutral'
  return dark ? DARK_PALETTES[normalized] : LIGHT_PALETTES[normalized]
}

function resolveLineStyle(style: unknown): TradingChartLineStyle {
  return typeof style === 'string' && VALID_STYLES.has(style as TradingChartLineStyle)
    ? (style as TradingChartLineStyle)
    : 'solid'
}

function isFiniteNumber(v: unknown): v is number {
  return typeof v === 'number' && Number.isFinite(v)
}

function validatePoint(raw: unknown, context: string): TradingChartPoint {
  if (!raw || typeof raw !== 'object') {
    throw new Error(`Invalid point in ${context}`)
  }
  const obj = raw as Record<string, unknown>
  if (!isFiniteNumber(obj.time) || !isFiniteNumber(obj.price)) {
    throw new Error(`Non-finite point coordinates in ${context}`)
  }
  return { time: obj.time, price: obj.price }
}

function validateAndNormalizeChartModel(raw: unknown, dark: boolean): NormalizedTradingChartModel {
  if (!raw || typeof raw !== 'object') {
    throw new Error('Chart document must be an object')
  }
  const doc = raw as Record<string, unknown>
  if (doc.schemaVersion !== 1) {
    throw new Error(`Unsupported chart schemaVersion: ${String(doc.schemaVersion)}`)
  }
  const id = typeof doc.id === 'string' && doc.id.trim() ? doc.id.trim() : 'chart'
  const title = typeof doc.title === 'string' && doc.title.trim() ? doc.title.trim() : id
  const symbol = typeof doc.symbol === 'string' && doc.symbol.trim() ? doc.symbol.trim() : 'PA-EXAMPLE'
  const timeframe = typeof doc.timeframe === 'string' && doc.timeframe.trim() ? doc.timeframe.trim() : '1h'
  const revision = isFiniteNumber(doc.revision) && doc.revision >= 1 ? Math.floor(doc.revision) : 1

  if (!Array.isArray(doc.candles) || doc.candles.length < 3 || doc.candles.length > 500) {
    throw new Error('Chart candles array must contain between 3 and 500 bars')
  }

  const candles: TradingChartCandle[] = []
  let prevTime = -Infinity
  for (let i = 0; i < doc.candles.length; i++) {
    const c = doc.candles[i] as Record<string, unknown> | null
    if (!c || typeof c !== 'object') {
      throw new Error(`Candle at index ${i} must be an object`)
    }
    const { time, open, high, low, close, volume, tag } = c
    if (
      !isFiniteNumber(time) ||
      !isFiniteNumber(open) ||
      !isFiniteNumber(high) ||
      !isFiniteNumber(low) ||
      !isFiniteNumber(close)
    ) {
      throw new Error(`Candle at index ${i} has non-finite OHLC/time`)
    }
    if (time <= prevTime) {
      throw new Error(`Candle time at index ${i} is not strictly ascending`)
    }
    const maxBody = Math.max(open, close)
    const minBody = Math.min(open, close)
    if (high + 1e-9 < maxBody || low - 1e-9 > minBody || high + 1e-9 < low) {
      throw new Error(`Candle at index ${i} violates OHLC bounds`)
    }
    if (volume !== undefined && (!isFiniteNumber(volume) || volume < 0)) {
      throw new Error(`Candle at index ${i} has invalid volume`)
    }
    prevTime = time
    candles.push({
      time,
      open,
      high,
      low,
      close,
      ...(isFiniteNumber(volume) ? { volume } : {}),
      ...(typeof tag === 'string' && tag.trim() ? { tag: tag.trim() } : {}),
    })
  }

  const rawDrawings = doc.drawings === undefined ? [] : doc.drawings
  if (!Array.isArray(rawDrawings) || rawDrawings.length > 50) {
    throw new Error('Chart drawings must be an array of at most 50 annotations')
  }

  const seenIds = new Set<string>()
  const drawings: NormalizedTradingChartDrawing[] = []
  for (let i = 0; i < rawDrawings.length; i++) {
    const d = rawDrawings[i] as Record<string, unknown> | null
    if (!d || typeof d !== 'object') {
      throw new Error(`Drawing at index ${i} must be an object`)
    }
    const drawingId = typeof d.id === 'string' && d.id.trim() ? d.id.trim() : `drawing-${i + 1}`
    if (seenIds.has(drawingId)) {
      throw new Error(`Duplicate drawing id: ${drawingId}`)
    }
    seenIds.add(drawingId)
    const palette = resolveTonePalette(d.tone, dark)

    if (d.type === 'horizontal-line') {
      if (!isFiniteNumber(d.price)) {
        throw new Error(`horizontal-line "${drawingId}" requires finite price`)
      }
      drawings.push({
        id: drawingId,
        type: 'horizontal-line',
        price: d.price,
        label: typeof d.label === 'string' ? d.label.trim().slice(0, 80) : '',
        style: resolveLineStyle(d.style),
        palette,
      })
    } else if (d.type === 'trend-line') {
      if (!Array.isArray(d.points) || d.points.length !== 2) {
        throw new Error(`trend-line "${drawingId}" requires exactly 2 points`)
      }
      const p1 = validatePoint(d.points[0], `trend-line "${drawingId}"[0]`)
      const p2 = validatePoint(d.points[1], `trend-line "${drawingId}"[1]`)
      const width = isFiniteNumber(d.width) ? Math.min(4, Math.max(1, d.width)) : 2
      drawings.push({
        id: drawingId,
        type: 'trend-line',
        points: [p1, p2],
        label: typeof d.label === 'string' ? d.label.trim().slice(0, 80) : '',
        style: resolveLineStyle(d.style),
        width,
        palette,
      })
    } else if (d.type === 'price-zone') {
      const pFrom = validatePoint(d.from, `price-zone "${drawingId}".from`)
      const pTo = validatePoint(d.to, `price-zone "${drawingId}".to`)
      drawings.push({
        id: drawingId,
        type: 'price-zone',
        from: {
          time: Math.min(pFrom.time, pTo.time),
          price: Math.min(pFrom.price, pTo.price),
        },
        to: {
          time: Math.max(pFrom.time, pTo.time),
          price: Math.max(pFrom.price, pTo.price),
        },
        label: typeof d.label === 'string' ? d.label.trim().slice(0, 80) : '',
        palette,
      })
    } else if (d.type === 'text') {
      const anchor = validatePoint(d.anchor, `text "${drawingId}".anchor`)
      const text = typeof d.text === 'string' ? d.text.trim().slice(0, 60) : ''
      if (!text) {
        throw new Error(`text drawing "${drawingId}" requires non-empty text`)
      }
      drawings.push({
        id: drawingId,
        type: 'text',
        anchor,
        text,
        palette,
      })
    } else {
      throw new Error(`Unsupported drawing type: ${String(d.type)}`)
    }
  }

  const rawDisplay =
    doc.display && typeof doc.display === 'object' ? (doc.display as Record<string, unknown>) : {}
  const height = isFiniteNumber(rawDisplay.height)
    ? Math.min(720, Math.max(240, Math.round(rawDisplay.height)))
    : 420
  const showVolume = typeof rawDisplay.showVolume === 'boolean' ? rawDisplay.showVolume : true
  const fitContent = typeof rawDisplay.fitContent === 'boolean' ? rawDisplay.fitContent : true

  return {
    id,
    title,
    symbol,
    timeframe,
    revision,
    dark,
    display: { height, showVolume, fitContent },
    candles,
    drawings,
  }
}

function parseFenceChartSrc(rawText: string): string {
  const parsed = JSON.parse(rawText.trim()) as Record<string, unknown>
  if (!parsed || typeof parsed !== 'object' || typeof parsed.src !== 'string') {
    throw new Error('trading-chart block requires {"src":"charts/<slug>.chart.json"}')
  }
  const src = parsed.src.trim()
  if (!SAFE_CHART_SRC_RE.test(src) || src.includes('..')) {
    throw new Error(`Invalid or unsafe chart path: ${src}`)
  }
  return src
}

async function defaultChartFetcher(src: string): Promise<unknown> {
  const match = SAFE_CHART_SRC_RE.exec(src)
  if (!match) {
    throw new Error(`Invalid chart path: ${src}`)
  }
  const fileName = match[1]!
  return requestJson<unknown>(`/api/admin/knowledge/charts/${encodeURIComponent(fileName)}`)
}

function formatPrice(value: number): string {
  const abs = Math.abs(value)
  if (abs >= 1000) return value.toFixed(1)
  if (abs >= 1) return value.toFixed(2)
  return value.toPrecision(4)
}

function dashArrayForStyle(style: TradingChartLineStyle): string | null {
  if (style === 'dashed') return '6 4'
  if (style === 'dotted') return '2 3'
  return null
}

const SVG_NS = 'http://www.w3.org/2000/svg'

const defaultChartEngineAdapter: ChartEngineAdapter = {
  mount(container, model) {
    container.innerHTML = ''
    container.style.height = `${model.display.height}px`

    const statusBar = document.createElement('div')
    statusBar.className = 'knowledge-trading-chart-statusbar'
    const lastBar = model.candles[model.candles.length - 1]!
    const setStatusFromBar = (bar: TradingChartCandle, index: number) => {
      const tagBadge = bar.tag ? `  •  #${bar.tag}` : ''
      statusBar.textContent = `Bar ${index + 1}/${model.candles.length}  O: ${formatPrice(bar.open)}  H: ${formatPrice(bar.high)}  L: ${formatPrice(bar.low)}  C: ${formatPrice(bar.close)}${bar.volume !== undefined ? `  Vol: ${Math.round(bar.volume)}` : ''}${tagBadge}`
    }
    setStatusFromBar(lastBar, model.candles.length - 1)
    container.appendChild(statusBar)

    const svg = document.createElementNS(SVG_NS, 'svg')
    svg.setAttribute('class', 'knowledge-trading-chart-svg')
    svg.setAttribute('role', 'img')
    svg.setAttribute('aria-label', `${model.title} (${model.symbol} ${model.timeframe})`)
    container.appendChild(svg)

    let viewStart = 0
    let viewEnd = model.candles.length - 1
    let hoverIndex: number | null = null

    const timeToIndex = (time: number): number => {
      const candles = model.candles
      if (time <= candles[0]!.time) return 0
      if (time >= candles[candles.length - 1]!.time) return candles.length - 1
      let lo = 0
      let hi = candles.length - 1
      while (lo <= hi) {
        const mid = (lo + hi) >> 1
        const midTime = candles[mid]!.time
        if (midTime === time) return mid
        if (midTime < time) lo = mid + 1
        else hi = mid - 1
      }
      const prevIdx = Math.max(0, lo - 1)
      const nextIdx = Math.min(candles.length - 1, lo)
      return Math.abs(candles[prevIdx]!.time - time) <= Math.abs(candles[nextIdx]!.time - time)
        ? prevIdx
        : nextIdx
    }

    const renderSvg = () => {
      const width = Math.max(360, container.clientWidth || 760)
      const height = Math.max(200, model.display.height - 28)
      svg.setAttribute('viewBox', `0 0 ${width} ${height}`)
      svg.innerHTML = ''

      const padLeft = 14
      const padRight = 68
      const padTop = 18
      const padBottom = 26
      const plotW = Math.max(120, width - padLeft - padRight)
      const plotH = Math.max(100, height - padTop - padBottom)

      const visibleSlice = model.candles.slice(viewStart, viewEnd + 1)
      let minPrice = Infinity
      let maxPrice = -Infinity
      let maxVolume = 0
      for (const c of visibleSlice) {
        if (c.low < minPrice) minPrice = c.low
        if (c.high > maxPrice) maxPrice = c.high
        if (c.volume && c.volume > maxVolume) maxVolume = c.volume
      }
      for (const d of model.drawings) {
        if (d.type === 'horizontal-line') {
          minPrice = Math.min(minPrice, d.price)
          maxPrice = Math.max(maxPrice, d.price)
        } else if (d.type === 'price-zone') {
          minPrice = Math.min(minPrice, d.from.price, d.to.price)
          maxPrice = Math.max(maxPrice, d.from.price, d.to.price)
        } else if (d.type === 'trend-line') {
          minPrice = Math.min(minPrice, d.points[0].price, d.points[1].price)
          maxPrice = Math.max(maxPrice, d.points[0].price, d.points[1].price)
        } else if (d.type === 'text') {
          minPrice = Math.min(minPrice, d.anchor.price)
          maxPrice = Math.max(maxPrice, d.anchor.price)
        }
      }
      if (!Number.isFinite(minPrice) || !Number.isFinite(maxPrice) || maxPrice <= minPrice) {
        minPrice = 0
        maxPrice = 100
      }
      const priceMargin = (maxPrice - minPrice) * 0.08 || 1
      const yMin = minPrice - priceMargin
      const yMax = maxPrice + priceMargin

      const visibleCount = Math.max(1, viewEnd - viewStart + 1)
      const stepX = plotW / visibleCount
      const xForIndex = (idx: number) => padLeft + (idx - viewStart + 0.5) * stepX
      const yForPrice = (price: number) =>
        padTop + ((yMax - price) / (yMax - yMin)) * plotH

      const gridColor = model.dark ? 'rgba(148, 163, 184, 0.12)' : 'rgba(100, 116, 139, 0.14)'
      const axisTextColor = model.dark ? '#94a3b8' : '#64748b'

      // 1. Horizontal price grid + right-axis labels
      const gridTicks = 5
      for (let t = 0; t <= gridTicks; t++) {
        const ratio = t / gridTicks
        const y = padTop + ratio * plotH
        const priceVal = yMax - ratio * (yMax - yMin)
        const line = document.createElementNS(SVG_NS, 'line')
        line.setAttribute('x1', String(padLeft))
        line.setAttribute('x2', String(padLeft + plotW))
        line.setAttribute('y1', String(y))
        line.setAttribute('y2', String(y))
        line.setAttribute('stroke', gridColor)
        line.setAttribute('stroke-width', '1')
        svg.appendChild(line)

        const label = document.createElementNS(SVG_NS, 'text')
        label.setAttribute('x', String(padLeft + plotW + 6))
        label.setAttribute('y', String(y + 4))
        label.setAttribute('fill', axisTextColor)
        label.setAttribute('font-size', '10')
        label.setAttribute('font-family', 'ui-monospace, SFMono-Regular, Menlo, monospace')
        label.textContent = formatPrice(priceVal)
        svg.appendChild(label)
      }

      // 2. Price zones (drawn behind candles so wicks/bodies stay crisp)
      for (const d of model.drawings) {
        if (d.type !== 'price-zone') continue
        const i1 = timeToIndex(d.from.time)
        const i2 = timeToIndex(d.to.time)
        const x1 = Math.max(padLeft, Math.min(xForIndex(i1), xForIndex(i2)) - stepX * 0.35)
        const x2 = Math.min(padLeft + plotW, Math.max(xForIndex(i1), xForIndex(i2)) + stepX * 0.35)
        const yTop = yForPrice(Math.max(d.from.price, d.to.price))
        const yBot = yForPrice(Math.min(d.from.price, d.to.price))
        if (x2 <= padLeft || x1 >= padLeft + plotW) continue

        const rect = document.createElementNS(SVG_NS, 'rect')
        rect.setAttribute('x', String(x1))
        rect.setAttribute('y', String(yTop))
        rect.setAttribute('width', String(Math.max(6, x2 - x1)))
        rect.setAttribute('height', String(Math.max(4, yBot - yTop)))
        rect.setAttribute('fill', d.palette.fill)
        rect.setAttribute('stroke', d.palette.stroke)
        rect.setAttribute('stroke-width', '1')
        rect.setAttribute('rx', '2')
        svg.appendChild(rect)

        if (d.label) {
          const text = document.createElementNS(SVG_NS, 'text')
          text.setAttribute('x', String(x1 + 6))
          text.setAttribute('y', String(yTop + 13))
          text.setAttribute('fill', d.palette.text)
          text.setAttribute('font-size', '11')
          text.setAttribute('font-weight', '600')
          text.textContent = d.label
          svg.appendChild(text)
        }
      }

      // 3. Volume bars (if enabled)
      if (model.display.showVolume && maxVolume > 0) {
        const volMaxH = plotH * 0.18
        for (let i = viewStart; i <= viewEnd; i++) {
          const c = model.candles[i]!
          if (!c.volume) continue
          const isBull = c.close >= c.open
          const cx = xForIndex(i)
          const bw = Math.max(2, Math.min(18, stepX * 0.56))
          const vh = Math.max(1, (c.volume / maxVolume) * volMaxH)
          const rect = document.createElementNS(SVG_NS, 'rect')
          rect.setAttribute('x', String(cx - bw / 2))
          rect.setAttribute('y', String(padTop + plotH - vh))
          rect.setAttribute('width', String(bw))
          rect.setAttribute('height', String(vh))
          rect.setAttribute(
            'fill',
            isBull
              ? model.dark
                ? 'rgba(52, 211, 153, 0.18)'
                : 'rgba(5, 150, 105, 0.15)'
              : model.dark
                ? 'rgba(248, 113, 113, 0.18)'
                : 'rgba(220, 38, 38, 0.15)',
          )
          svg.appendChild(rect)
        }
      }

      // 4. Candlesticks (wicks + bodies)
      const bullColor = model.dark ? '#10b981' : '#059669'
      const bearColor = model.dark ? '#ef4444' : '#dc2626'
      const bodyW = Math.max(3, Math.min(20, stepX * 0.68))

      for (let i = viewStart; i <= viewEnd; i++) {
        const c = model.candles[i]!
        const isBull = c.close >= c.open
        const color = isBull ? bullColor : bearColor
        const cx = xForIndex(i)
        const yHigh = yForPrice(c.high)
        const yLow = yForPrice(c.low)
        const yOpen = yForPrice(c.open)
        const yClose = yForPrice(c.close)

        const wick = document.createElementNS(SVG_NS, 'line')
        wick.setAttribute('x1', String(cx))
        wick.setAttribute('x2', String(cx))
        wick.setAttribute('y1', String(yHigh))
        wick.setAttribute('y2', String(yLow))
        wick.setAttribute('stroke', color)
        wick.setAttribute('stroke-width', '1.4')
        svg.appendChild(wick)

        const bodyTop = Math.min(yOpen, yClose)
        const bodyHeight = Math.max(2, Math.abs(yClose - yOpen))
        const body = document.createElementNS(SVG_NS, 'rect')
        body.setAttribute('x', String(cx - bodyW / 2))
        body.setAttribute('y', String(bodyTop))
        body.setAttribute('width', String(bodyW))
        body.setAttribute('height', String(bodyHeight))
        body.setAttribute('fill', color)
        body.setAttribute('rx', '1')
        svg.appendChild(body)
      }

      // 5. Horizontal lines, trend-lines, and text callouts
      for (const d of model.drawings) {
        if (d.type === 'horizontal-line') {
          const y = yForPrice(d.price)
          const line = document.createElementNS(SVG_NS, 'line')
          line.setAttribute('x1', String(padLeft))
          line.setAttribute('x2', String(padLeft + plotW))
          line.setAttribute('y1', String(y))
          line.setAttribute('y2', String(y))
          line.setAttribute('stroke', d.palette.stroke)
          line.setAttribute('stroke-width', '1.5')
          const dash = dashArrayForStyle(d.style)
          if (dash) line.setAttribute('stroke-dasharray', dash)
          svg.appendChild(line)

          if (d.label) {
            const label = document.createElementNS(SVG_NS, 'text')
            label.setAttribute('x', String(padLeft + plotW - 6))
            label.setAttribute('y', String(y - 5))
            label.setAttribute('text-anchor', 'end')
            label.setAttribute('fill', d.palette.text)
            label.setAttribute('font-size', '11')
            label.setAttribute('font-weight', '600')
            label.textContent = d.label
            svg.appendChild(label)
          }
        } else if (d.type === 'trend-line') {
          const i1 = timeToIndex(d.points[0].time)
          const i2 = timeToIndex(d.points[1].time)
          const x1 = xForIndex(i1)
          const x2 = xForIndex(i2)
          const y1 = yForPrice(d.points[0].price)
          const y2 = yForPrice(d.points[1].price)

          const line = document.createElementNS(SVG_NS, 'line')
          line.setAttribute('x1', String(x1))
          line.setAttribute('y1', String(y1))
          line.setAttribute('x2', String(x2))
          line.setAttribute('y2', String(y2))
          line.setAttribute('stroke', d.palette.stroke)
          line.setAttribute('stroke-width', String(d.width))
          const dash = dashArrayForStyle(d.style)
          if (dash) line.setAttribute('stroke-dasharray', dash)
          svg.appendChild(line)

          if (d.label) {
            const midX = (x1 + x2) / 2
            const midY = (y1 + y2) / 2 - 6
            const label = document.createElementNS(SVG_NS, 'text')
            label.setAttribute('x', String(midX))
            label.setAttribute('y', String(midY))
            label.setAttribute('text-anchor', 'middle')
            label.setAttribute('fill', d.palette.text)
            label.setAttribute('font-size', '11')
            label.setAttribute('font-weight', '600')
            label.textContent = d.label
            svg.appendChild(label)
          }
        } else if (d.type === 'text') {
          const idx = timeToIndex(d.anchor.time)
          const cx = Math.min(padLeft + plotW - 40, Math.max(padLeft + 40, xForIndex(idx)))
          const cy = Math.min(padTop + plotH - 16, Math.max(padTop + 18, yForPrice(d.anchor.price)))

          const dot = document.createElementNS(SVG_NS, 'circle')
          dot.setAttribute('cx', String(xForIndex(idx)))
          dot.setAttribute('cy', String(yForPrice(d.anchor.price)))
          dot.setAttribute('r', '3.5')
          dot.setAttribute('fill', d.palette.stroke)
          svg.appendChild(dot)

          const label = document.createElementNS(SVG_NS, 'text')
          label.setAttribute('x', String(cx))
          label.setAttribute('y', String(cy - 8))
          label.setAttribute('text-anchor', 'middle')
          label.setAttribute('fill', d.palette.text)
          label.setAttribute('font-size', '11')
          label.setAttribute('font-weight', '600')
          label.textContent = d.text
          svg.appendChild(label)
        }
      }

      // 6. Hover crosshair
      if (hoverIndex !== null && hoverIndex >= viewStart && hoverIndex <= viewEnd) {
        const cx = xForIndex(hoverIndex)
        const vLine = document.createElementNS(SVG_NS, 'line')
        vLine.setAttribute('x1', String(cx))
        vLine.setAttribute('x2', String(cx))
        vLine.setAttribute('y1', String(padTop))
        vLine.setAttribute('y2', String(padTop + plotH))
        vLine.setAttribute('stroke', axisTextColor)
        vLine.setAttribute('stroke-width', '1')
        vLine.setAttribute('stroke-dasharray', '3 3')
        svg.appendChild(vLine)
      }
    }

    let dragOriginX: number | null = null
    let dragStartRange: [number, number] = [0, model.candles.length - 1]

    const onPointerDown = (e: MouseEvent) => {
      if (e.button !== 0) return
      dragOriginX = e.clientX
      dragStartRange = [viewStart, viewEnd]
    }

    const onPointerUp = () => {
      dragOriginX = null
    }

    const onPointerMove = (e: MouseEvent) => {
      const rect = svg.getBoundingClientRect()
      if (!rect.width) return
      const padLeft = 14
      const padRight = 68
      const plotW = Math.max(120, rect.width - padLeft - padRight)
      const visibleCount = Math.max(1, viewEnd - viewStart + 1)

      if (dragOriginX !== null && visibleCount < model.candles.length) {
        const dx = e.clientX - dragOriginX
        const barShift = Math.round((-dx / plotW) * visibleCount)
        const span = dragStartRange[1] - dragStartRange[0] + 1
        const nextStart = Math.max(
          0,
          Math.min(model.candles.length - span, dragStartRange[0] + barShift),
        )
        viewStart = nextStart
        viewEnd = nextStart + span - 1
        renderSvg()
        return
      }

      const relX = e.clientX - rect.left - padLeft
      const idx = viewStart + Math.floor((relX / plotW) * visibleCount)
      if (idx >= viewStart && idx <= viewEnd && model.candles[idx]) {
        hoverIndex = idx
        setStatusFromBar(model.candles[idx]!, idx)
        renderSvg()
      }
    }

    const onPointerLeave = () => {
      dragOriginX = null
      hoverIndex = null
      setStatusFromBar(lastBar, model.candles.length - 1)
      renderSvg()
    }

    const onWheel = (e: WheelEvent) => {
      if (model.candles.length <= 5) return
      e.preventDefault()
      const currentSpan = viewEnd - viewStart + 1
      const delta = e.deltaY > 0 ? Math.ceil(currentSpan * 0.15) : -Math.ceil(currentSpan * 0.15)
      const nextSpan = Math.min(model.candles.length, Math.max(5, currentSpan + delta))
      const center = Math.round((viewStart + viewEnd) / 2)
      viewStart = Math.max(0, Math.min(model.candles.length - nextSpan, center - Math.floor(nextSpan / 2)))
      viewEnd = Math.min(model.candles.length - 1, viewStart + nextSpan - 1)
      renderSvg()
    }

    svg.addEventListener('mousedown', onPointerDown)
    svg.addEventListener('mouseup', onPointerUp)
    svg.addEventListener('mousemove', onPointerMove)
    svg.addEventListener('mouseleave', onPointerLeave)
    svg.addEventListener('wheel', onWheel, { passive: false })

    let resizeObserver: ResizeObserver | null = null
    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(() => renderSvg())
      resizeObserver.observe(container)
    }

    renderSvg()

    return {
      fitContent() {
        viewStart = 0
        viewEnd = model.candles.length - 1
        hoverIndex = null
        setStatusFromBar(lastBar, model.candles.length - 1)
        renderSvg()
      },
      destroy() {
        svg.removeEventListener('mousedown', onPointerDown)
        svg.removeEventListener('mouseup', onPointerUp)
        svg.removeEventListener('mousemove', onPointerMove)
        svg.removeEventListener('mouseleave', onPointerLeave)
        svg.removeEventListener('wheel', onWheel)
        resizeObserver?.disconnect()
        container.innerHTML = ''
      },
    }
  },
}

function buildChartWidgetDom(
  src: string,
  rawDoc: unknown,
  model: NormalizedTradingChartModel,
  engineAdapter: ChartEngineAdapter,
): HTMLElement {
  const widget = document.createElement('div')
  widget.className = 'knowledge-trading-chart-widget'
  widget.dataset.tradingChartSrc = src
  widget.dataset.tradingChartTheme = model.dark ? 'dark' : 'light'

  const header = document.createElement('div')
  header.className = 'knowledge-trading-chart-header'

  const titleGroup = document.createElement('div')
  titleGroup.className = 'knowledge-trading-chart-title-group'

  const titleEl = document.createElement('span')
  titleEl.className = 'knowledge-trading-chart-title'
  titleEl.textContent = model.title
  titleGroup.appendChild(titleEl)

  const badgeEl = document.createElement('span')
  badgeEl.className = 'knowledge-trading-chart-badge'
  badgeEl.textContent = `${model.symbol} · ${model.timeframe} · ${model.candles.length} bars`
  titleGroup.appendChild(badgeEl)

  header.appendChild(titleGroup)

  const fitBtn = document.createElement('button')
  fitBtn.type = 'button'
  fitBtn.className = 'knowledge-trading-chart-fit-btn'
  fitBtn.dataset.chartAction = 'fit-content'
  fitBtn.textContent = 'Fit Content'
  header.appendChild(fitBtn)

  widget.appendChild(header)

  const viewport = document.createElement('div')
  viewport.className = 'knowledge-trading-chart-viewport'
  widget.appendChild(viewport)

  if (model.drawings.length > 0) {
    const legend = document.createElement('div')
    legend.className = 'knowledge-trading-chart-legend'
    for (const d of model.drawings) {
      const labelText = d.type === 'text' ? d.text : d.label
      if (!labelText) continue
      const item = document.createElement('span')
      item.className = 'knowledge-trading-chart-legend-item'
      const swatch = document.createElement('span')
      swatch.className = 'knowledge-trading-chart-legend-swatch'
      swatch.style.backgroundColor = d.palette.stroke
      item.appendChild(swatch)
      const textSpan = document.createElement('span')
      textSpan.textContent = labelText
      item.appendChild(textSpan)
      legend.appendChild(item)
    }
    if (legend.childElementCount > 0) {
      widget.appendChild(legend)
    }
  }

  const instance = engineAdapter.mount(viewport, model)
  fitBtn.addEventListener('click', () => {
    instance.fitContent()
  })

  activeWidgets.set(widget, {
    instance,
    rawDoc,
    dark: model.dark,
  })

  return widget
}

function markBlockError(block: HTMLElement, message: string): void {
  block.dataset.tradingChartSource = 'error'
  block.dataset.tradingChartError = message
}

export async function renderTradingChartBlocks(
  root: HTMLElement | null,
  dark: boolean,
  deps: TradingChartRendererDeps = {},
): Promise<void> {
  if (!root) return
  const fetcher = deps.fetcher ?? defaultChartFetcher
  const engineAdapter = deps.engineAdapter ?? defaultChartEngineAdapter

  // 1. Re-hydrate already mounted widgets if `dark` theme changed
  const existingWidgets = Array.from(
    root.querySelectorAll<HTMLElement>('.knowledge-trading-chart-widget[data-trading-chart-src]'),
  )
  for (const widget of existingWidgets) {
    const state = activeWidgets.get(widget)
    if (!state || state.dark === dark) continue
    const src = widget.dataset.tradingChartSrc || ''
    state.instance.destroy()
    activeWidgets.delete(widget)
    try {
      const nextModel = validateAndNormalizeChartModel(state.rawDoc, dark)
      const replacement = buildChartWidgetDom(src, state.rawDoc, nextModel, engineAdapter)
      if (root.contains(widget)) {
        widget.replaceWith(replacement)
      }
    } catch {
      // Leave widget disconnected if re-normalization fails
    }
  }

  // 2. Hydrate unrendered `<pre class="knowledge-trading-chart" data-trading-chart-source="true">` blocks
  const blocks = Array.from(
    root.querySelectorAll<HTMLElement>('pre.knowledge-trading-chart[data-trading-chart-source="true"]'),
  )
  if (!blocks.length) return

  for (const block of blocks) {
    if (!root.contains(block) || block.dataset.tradingChartSource !== 'true') continue
    const sourceText = block.textContent || ''
    let src: string
    try {
      src = parseFenceChartSrc(sourceText)
    } catch (err) {
      markBlockError(block, err instanceof Error ? err.message : 'Invalid trading-chart block')
      continue
    }

    try {
      const rawDoc = await fetcher(src)
      if (!root.contains(block) || block.textContent !== sourceText) continue
      const model = validateAndNormalizeChartModel(rawDoc, dark)
      const widget = buildChartWidgetDom(src, rawDoc, model, engineAdapter)
      block.replaceWith(widget)
    } catch (err) {
      markBlockError(block, err instanceof Error ? err.message : 'Failed to load trading chart')
    }
  }
}
