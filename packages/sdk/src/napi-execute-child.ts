import { Akua } from './mod.ts';

const exitCode = await new Akua().execute(['version', '--json']);
console.log(exitCode);
