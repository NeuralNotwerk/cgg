import { spawnJob } from './zzz/svc2';
const legacy = require('./aaa/svc');
export function useNamed() { spawnJob(3); legacy.spawnJob(4); }
