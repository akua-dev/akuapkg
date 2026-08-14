import { execute } from './execute.ts';

const exitCode = execute(['version', '--json'], { binName: 'akua pkg' });
console.log(exitCode);
