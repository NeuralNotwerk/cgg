import axios from 'axios';
import * as _ from 'lodash';
import { promises as fsp } from 'fs';
import * as fs from 'fs';
import mongoose from 'mongoose';

export async function main(): Promise<void> {
  await axios.post('/u', {});
  _.debounce(main, 10);
  fs.readFile('x', () => {});
  await fsp.readFile('x');
  await mongoose.connect('mongodb://x');
}
