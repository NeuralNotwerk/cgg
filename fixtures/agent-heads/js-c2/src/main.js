import * as svc from './svc';
const legacy = require('./legacy');
export function go() {
  svc.spawnJob(1);
  legacy.legacyRun(2);
}
