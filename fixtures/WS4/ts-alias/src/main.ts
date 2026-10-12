import { fmt } from '@/lib/util';
import * as missing from './not-analyzed';
export function go() {
  fmt("a");
  missing.gone();
}
