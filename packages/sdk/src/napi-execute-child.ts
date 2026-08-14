import { Akua } from './mod.ts';

const exitCode = await new Akua().execute(['version', '--json']);
console.log(exitCode);

const helpExitCode = await new Akua().execute(['render', '--help'], {
	binName: 'akua pkg',
});
console.log(helpExitCode);
