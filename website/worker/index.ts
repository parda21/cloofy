// cloofy.app: Workers static assets serve the site; this Worker only runs for the install
// script, so `curl -fsSL https://cloofy.app/install.sh | bash` gets the installer on main.

interface Env {
	ASSETS: Fetcher;
}

const INSTALLER = 'https://raw.githubusercontent.com/parda21/cloofy/main/install.sh';

export default {
	async fetch(request, env): Promise<Response> {
		const url = new URL(request.url);
		if (url.pathname === '/install.sh' || url.pathname === '/install') {
			return installer();
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
