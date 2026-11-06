import { describe, expect, it } from 'vitest';
import { parseFaces } from './NeutralPrompt';

describe('parseFaces', () => {
  it('accepts spaces and commas', () => {
    expect(parseFaces('4 6 6')).toEqual([4, 6, 6]);
    expect(parseFaces(' 1, 2,3 ')).toEqual([1, 2, 3]);
    expect(parseFaces('')).toEqual([]);
  });
});
