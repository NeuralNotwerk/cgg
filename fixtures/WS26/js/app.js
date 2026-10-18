import * as util from './util.js';
import { sorter } from './util.js';

function mapper(x) { return x * 2; }
function tick() { console.log('tick'); }
function onData(d) { return d; }
function inTable() { return 1; }
function shorthandFn() { return 2; }
function returned() { return 3; }
function assignedCb() { return 4; }
function fallbackFn() { return 5; }
function orphan() { return 6; }          // positive control: truly dead
function shadowed() { return 7; }        // near-miss: only a parameter of this name is passed
function inString() { return 8; }        // near-miss: only a string of this name
function asKey() { return 9; }           // near-miss: only a property key of this name

class Widget {
  handleClick() { return 10; }
  neverUsed() { return 11; }
  handleOther() { return 12; }
  start(emitter) {
    emitter.on('click', this.handleClick);
    [1].forEach(this.handleOther);
  }
}

function makeHandler() {
  return returned;
}

function useParam(shadowed) {
  [1, 2].forEach(shadowed);
}

export function main(emitter, obj) {
  [1, 2, 3].map(mapper);
  [3, 1].sort(sorter);
  [1].map(util.formatter);
  setTimeout(tick, 10);
  emitter.on('data', onData);
  const table = { a: inTable, shorthandFn };
  obj.cb = assignedCb;
  const pick = obj.f || fallbackFn;
  console.log('inString');
  const o = { asKey: 1 };
  new Widget().start(emitter);
  makeHandler();
  useParam(1);
  return [table, pick, o];
}
