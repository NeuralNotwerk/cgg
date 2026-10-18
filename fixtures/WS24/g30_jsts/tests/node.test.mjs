import test from 'node:test';
import assert from 'node:assert';
import { nodeHelper } from '../src/nh.mjs';
test('node runner', () => { assert.ok(nodeHelper()); });
