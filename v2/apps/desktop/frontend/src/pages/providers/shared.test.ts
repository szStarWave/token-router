/**
 * providers/shared 单测：header 文本解析（编辑面板提交语义）。
 */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { formatRelativeTime, parseHeaders } from './shared'

test('parseHeaders：每行 Name: Value；坏行丢弃；空输入为空数组', () => {
  assert.deepEqual(
    parseHeaders('HTTP-Referer: https://example.com\nX-Title: app'),
    [
      { name: 'HTTP-Referer', value: 'https://example.com' },
      { name: 'X-Title', value: 'app' },
    ],
  )
  assert.deepEqual(parseHeaders(''), [])
  assert.deepEqual(parseHeaders('not-a-header'), [])
  assert.deepEqual(parseHeaders('\n\n  \n'), [])
})

test('formatRelativeTime：粗粒度、未来时间按刚刚处理', () => {
  const now = 1_000_000_000
  assert.equal(formatRelativeTime(now, now), '刚刚')
  assert.equal(formatRelativeTime(now - 5 * 60_000, now), '5 分钟前')
  assert.equal(formatRelativeTime(now - 3 * 3_600_000, now), '3 小时前')
  assert.equal(formatRelativeTime(now - 50 * 3_600_000, now), '2 天前')
  assert.equal(formatRelativeTime(now + 5_000, now), '刚刚')
})
