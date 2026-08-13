import { Akua, configureNapi } from './mod.ts';
import { loadNapi, type NapiAddon } from './napi.ts';

const addon = {
	version: () => ({ version: 'embedded' }),
	execute: (args: string[]) => (args.join(' ') === 'version' ? 0 : 2),
} as NapiAddon;

configureNapi(addon);
console.log((loadNapi().version() as { version: string }).version);
console.log(await new Akua().execute(['version']));
