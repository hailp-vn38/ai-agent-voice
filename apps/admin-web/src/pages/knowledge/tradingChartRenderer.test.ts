import { describe, expect, it, vi } from 'vitest'
import { renderMarkdown } from '@/lib/markdown'
import {
  type ChartEngineAdapter,
  type NormalizedTradingChartModel,
  renderTradingChartBlocks,
  type TradingChartDocument,
} from './tradingChartRenderer'

function makeSampleChartDocument(overrides: Partial<TradingChartDocument> = {}): TradingChartDocument {
  return {
    schemaVersion: 1,
    revision: 2,
    id: 'bullish-ob-fvg-example',
    title: 'Minh họa Bullish Order Block + FVG & Retest Entry',
    symbol: 'PA-EXAMPLE',
    timeframe: '1h',
    display: {
      height: 420,
      showVolume: true,
      fitContent: true,
    },
    candles: [
      { time: 1704067200, open: 104.2, high: 105.0, low: 103.5, close: 104.8, volume: 120 },
      { time: 1704070800, open: 104.8, high: 108.0, low: 104.5, close: 107.6, volume: 210, tag: 'swing-high' },
      { time: 1704074400, open: 107.6, high: 107.8, low: 101.5, close: 103.0, volume: 180, tag: 'ob-candle' },
      { time: 1704078000, open: 103.0, high: 109.5, low: 102.9, close: 109.2, volume: 420, tag: 'impulse-mid' },
      { time: 1704081600, open: 109.2, high: 113.5, low: 105.2, close: 113.0, volume: 380, tag: 'bos-peak' },
    ],
    drawings: [
      {
        id: 'bos-line',
        type: 'trend-line',
        points: [
          { time: 1704070800, price: 108.0 },
          { time: 1704081600, price: 108.0 },
        ],
        label: 'BOS (108.0)',
        tone: 'structure',
        style: 'dashed',
        width: 2,
      },
      {
        id: 'reversed-ob-zone',
        type: 'price-zone',
        // Reversed coordinates to verify normalization (from.time > to.time, from.price > to.price)
        from: { time: 1704081600, price: 103.0 },
        to: { time: 1704074400, price: 101.5 },
        label: 'Bullish Order Block (101.5 - 103.0)',
        tone: 'demand',
      },
      {
        id: 'tp-level',
        type: 'horizontal-line',
        price: 114.0,
        label: 'Take Profit (114.0)',
        tone: 'bullish',
        style: 'dotted',
      },
      {
        id: 'entry-note',
        type: 'text',
        anchor: { time: 1704074400, price: 101.5 },
        text: 'Pin Bar Retest (Entry)',
        tone: 'warning',
      },
    ],
    ...overrides,
  }
}

describe('renderTradingChartBlocks', () => {
  it('hydrates trading-chart block, normalizes price-zone bounds, maps tones by theme, and wires Fit Content', async () => {
    const root = document.createElement('div')
    root.innerHTML = renderMarkdown([
      '# Lesson',
      '',
      '```trading-chart',
      '{"src":"charts/bullish-ob-fvg-example.chart.json"}',
      '```',
    ].join('\n'))

    const mountedModels: NormalizedTradingChartModel[] = []
    const fitContentSpy = vi.fn()
    const destroySpy = vi.fn()

    const engineAdapter: ChartEngineAdapter = {
      mount(container, model) {
        mountedModels.push(model)
        container.dataset.mountedChartId = model.id
        return {
          fitContent: fitContentSpy,
          destroy: destroySpy,
        }
      },
    }

    const fetcher = vi.fn(async (src: string) => {
      expect(src).toBe('charts/bullish-ob-fvg-example.chart.json')
      return makeSampleChartDocument()
    })

    await renderTradingChartBlocks(root, false, { fetcher, engineAdapter })

    expect(fetcher).toHaveBeenCalledTimes(1)
    expect(mountedModels).toHaveLength(1)
    const model = mountedModels[0]!
    expect(model.id).toBe('bullish-ob-fvg-example')
    expect(model.dark).toBe(false)

    // Verify reversed price-zone coordinates are normalized
    const zone = model.drawings.find(d => d.id === 'reversed-ob-zone')
    expect(zone?.type).toBe('price-zone')
    if (zone?.type === 'price-zone') {
      expect(zone.from).toEqual({ time: 1704074400, price: 101.5 })
      expect(zone.to).toEqual({ time: 1704081600, price: 103.0 })
      expect(zone.palette.tone).toBe('demand')
    }

    // Verify header and Fit Content button exist and trigger fitContent()
    const widget = root.querySelector<HTMLElement>('.knowledge-trading-chart-widget')
    expect(widget).not.toBeNull()
    expect(widget?.textContent).toContain('Minh họa Bullish Order Block + FVG & Retest Entry')
    expect(widget?.textContent).toContain('1h')

    const fitBtn = widget?.querySelector<HTMLButtonElement>('[data-chart-action="fit-content"]')
    expect(fitBtn).not.toBeNull()
    fitBtn?.click()
    expect(fitContentSpy).toHaveBeenCalledTimes(1)

    // Re-render with dark=true: should destroy previous instance and re-mount with dark palette
    await renderTradingChartBlocks(root, true, { fetcher, engineAdapter })
    expect(destroySpy).toHaveBeenCalledTimes(1)
    expect(mountedModels).toHaveLength(2)
    expect(mountedModels[1]!.dark).toBe(true)
    expect(mountedModels[1]!.drawings[0]!.palette.stroke).not.toBe(
      mountedModels[0]!.drawings[0]!.palette.stroke,
    )
  })

  it('rejects path traversal and unsafe src values without invoking fetcher', async () => {
    const root = document.createElement('div')
    root.innerHTML = renderMarkdown([
      '```trading-chart',
      '{"src":"charts/../secrets.chart.json"}',
      '```',
      '',
      '```trading-chart',
      '{"src":"https://evil.example.com/x.chart.json"}',
      '```',
    ].join('\n'))

    const fetcher = vi.fn()
    await renderTradingChartBlocks(root, false, { fetcher })

    expect(fetcher).not.toHaveBeenCalled()
    const errorBlocks = root.querySelectorAll('pre.knowledge-trading-chart[data-trading-chart-source="error"]')
    expect(errorBlocks).toHaveLength(2)
  })

  it('rejects invalid candle invariants and preserves sibling valid charts', async () => {
    const root = document.createElement('div')
    root.innerHTML = renderMarkdown([
      '```trading-chart',
      '{"src":"charts/broken-ohlc.chart.json"}',
      '```',
      '',
      '```trading-chart',
      '{"src":"charts/valid-setup.chart.json"}',
      '```',
    ].join('\n'))

    const fetcher = vi.fn(async (src: string) => {
      if (src.includes('broken-ohlc')) {
        return makeSampleChartDocument({
          id: 'broken-ohlc',
          candles: [
            // Invalid OHLC: high (102) < open (104)
            { time: 1704067200, open: 104.0, high: 102.0, low: 101.0, close: 103.0 },
            { time: 1704070800, open: 103.0, high: 105.0, low: 102.0, close: 104.0 },
            { time: 1704074400, open: 104.0, high: 106.0, low: 103.0, close: 105.0 },
          ],
        })
      }
      return makeSampleChartDocument({ id: 'valid-setup' })
    })

    await renderTradingChartBlocks(root, true, { fetcher })

    const errorBlock = root.querySelector('pre.knowledge-trading-chart[data-trading-chart-source="error"]')
    expect(errorBlock).not.toBeNull()

    const validWidget = root.querySelector('.knowledge-trading-chart-widget')
    expect(validWidget).not.toBeNull()
    // Default SVG/Canvas engineAdapter renders an SVG chart inside the viewport
    expect(validWidget?.querySelector('svg')).not.toBeNull()
  })

  it('rejects non-ascending candle time, unsupported schemaVersion, unknown drawing type, and fetcher failure', async () => {
    const root = document.createElement('div')
    root.innerHTML = renderMarkdown([
      '```trading-chart',
      '{"src":"charts/bad-time.chart.json"}',
      '```',
      '',
      '```trading-chart',
      '{"src":"charts/bad-schema.chart.json"}',
      '```',
      '',
      '```trading-chart',
      '{"src":"charts/bad-drawing.chart.json"}',
      '```',
      '',
      '```trading-chart',
      '{"src":"charts/network-fail.chart.json"}',
      '```',
    ].join('\n'))

    const fetcher = vi.fn(async (src: string) => {
      if (src.includes('bad-time')) {
        return makeSampleChartDocument({
          candles: [
            { time: 1704070800, open: 100, high: 105, low: 99, close: 102 },
            { time: 1704067200, open: 102, high: 106, low: 101, close: 104 },
            { time: 1704074400, open: 104, high: 108, low: 103, close: 107 },
          ],
        })
      }
      if (src.includes('bad-schema')) {
        return { ...makeSampleChartDocument(), schemaVersion: 99 }
      }
      if (src.includes('bad-drawing')) {
        return {
          ...makeSampleChartDocument(),
          drawings: [{ id: 'd1', type: 'script-injection' }],
        }
      }
      throw new Error('404 Not Found')
    })

    await renderTradingChartBlocks(root, false, { fetcher })

    const errorBlocks = root.querySelectorAll('pre.knowledge-trading-chart[data-trading-chart-source="error"]')
    expect(errorBlocks).toHaveLength(4)
  })
})
