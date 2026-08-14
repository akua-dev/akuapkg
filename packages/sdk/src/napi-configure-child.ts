import { Akua, configureNapi } from './mod.ts';
import { loadNapi, type NapiAddon } from './napi.ts';

const calls: Array<{ args: string[]; options?: { binName?: string } }> = [];
const addon = {
	version: () => ({ version: 'embedded' }),
	execute: (args: string[], options?: { binName?: string }) => {
		calls.push({ args, options });
		return args.join(' ') === 'version' ? 0 : 2;
	},
} as NapiAddon;

configureNapi(addon);
console.log((loadNapi().version() as { version: string }).version);
console.log(await new Akua().execute(['version'], { binName: 'akua pkg' }));
console.log(JSON.stringify(calls));
