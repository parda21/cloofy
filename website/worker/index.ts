// cloofy.app: Workers static assets serve the site; this Worker only runs for the install
// script, so `curl -fsSL https://cloofy.app/install.sh | bash` gets the installer on main,
// and for the star count on the landing page.

interface Env {
	ASSETS: Fetcher;
}

const INSTALLER = 'https://raw.githubusercontent.com/parda21/cloofy/main/install.sh';
const REPOSITORY = 'https://api.github.com/repos/parda21/cloofy';

export default {
	async fetch(request, env): Promise<Response> {
		const url = new URL(request.url);
		if (url.pathname === '/install.sh' || url.pathname === '/install') {
			return installer();
		}
		if (url.pathname === '/stars.json') {
			return stars();
		}
		return env.ASSETS.fetch(request);
	},
} satisfies ExportedHandler<Env>;

async function installer(): Promise<Response> {
	try {
		// GitHub's raw files change within minutes of a push; five minutes at the edge is close enough.
		const upstream = await fetch(INSTALLER, { cf: { cacheTtl: 300, cacheEverything: true } });
		if (upstream.ok) {
			return new Response(upstream.body, {
				headers: {
					'content-type': 'text/x-shellscript; charset=utf-8',
					'cache-control': 'public, max-age=300',
					'x-content-type-options': 'nosniff',
				},
			});
		}
	} catch {
		// Falls through to GitHub itself, which curl -L follows.
	}
	return Response.redirect(INSTALLER, 302);
}

/** The repository's stars. GitHub answers 60 requests an hour without a token, so the edge
 * keeps its answer for an hour, and visitors never ask GitHub themselves. */
async function stars(): Promise<Response> {
	let count: number | null = null;
	try {
		const upstream = await fetch(REPOSITORY, {
			headers: { 'user-agent': 'cloofy.app', accept: 'application/vnd.github+json' },
			cf: { cacheTtl: 3600, cacheEverything: true },
		});
		if (upstream.ok) count = ((await upstream.json()) as { stargazers_count?: number }).stargazers_count ?? null;
	} catch {
		// The page keeps the count it was built with.
	}
	return Response.json({ stars: count }, { headers: { 'cache-control': `public, max-age=${count === null ? 60 : 3600}` } });
}
