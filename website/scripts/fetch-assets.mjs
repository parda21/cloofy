// The site's pictures live on the repository's `assets` branch, like the README's, so clones of
// main stay small. This fetches the ones missing into public/ before a build.
import { mkdir, stat, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const base = 'https://raw.githubusercontent.com/parda21/cloofy/assets/';
const files = {
	'screenshots/hero.webp': 'hero.webp',
	'screenshots/home.webp': 'home.webp',
	'screenshots/nowplaying.webp': 'nowplaying.webp',
	'screenshots/playlist.webp': 'playlist.webp',
	'screenshots/artist.webp': 'artist.webp',
	'screenshots/search.webp': 'search.webp',
	'screenshots/library.webp': 'library.webp',
	'screenshots/palette.webp': 'palette.webp',
	'og.png': 'og.png',
};

const publicDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'public');
const exists = (path) => stat(path).then(() => true, () => false);

for (const [target, source] of Object.entries(files)) {
	const path = join(publicDir, target);
	if (await exists(path)) continue;
	const response = await fetch(base + source);
	if (!response.ok) throw new Error(`${source}: ${response.status} from the assets branch`);
	await mkdir(dirname(path), { recursive: true });
	await writeFile(path, Buffer.from(await response.arrayBuffer()));
	console.log(`fetched ${target}`);
}
