/**
 * Default interactive SVG/DOM `ChartEngineAdapter` for `tradingChartRenderer`.
 *
 * Renders OHLCV candlesticks, volume histogram, price zones, horizontal levels,
 * trend lines, text callouts, hover crosshair, wheel zoom, drag-to-pan, and
 * responsive ResizeObserver scaling without raw HTML injection.
 */
import type {
  ChartEngineAdapter,
  TradingChartCandle,
  TradingChartLineStyle,
} from './tradingChartRenderer'

const SVG_NS = 'http://www.w3.org/2000/svg'

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

export const defaultChartEngineAdapter: ChartEngineAdapter = {
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
