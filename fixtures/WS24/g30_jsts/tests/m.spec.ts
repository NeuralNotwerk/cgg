import { test, expect } from '@playwright/test';
import { helperPw } from '../src/pw';
test('loads', async ({ page }) => {
  await helperPw(page);
});
test.describe('group', () => {
  test('inner', async () => { await helperPw(null); });
});
