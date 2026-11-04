// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

const repo = 'https://github.com/parda21/cloofy';

// The landing page is `src/pages/index.astro`; Starlight serves the docs under /docs/.
export default defineConfig({
	site: 'https://cloofy.app',
	trailingSlash: 'ignore',
	integrations: [
		starlight({
			title: 'cloofy',
			description: 'Spotify and YouTube Music in your terminal.',
			logo: { src: './src/assets/logo.svg', replacesTitle: true },
			favicon: '/favicon.svg',
			social: [{ icon: 'github', label: 'GitHub', href: repo }],
			editLink: { baseUrl: `${repo}/edit/main/website/` },
			customCss: ['@fontsource-variable/jetbrains-mono', './src/styles/docs.css'],
			head: [
				{ tag: 'meta', attrs: { property: 'og:image', content: 'https://cloofy.app/og.png' } },
				{ tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
				{ tag: 'meta', attrs: { name: 'theme-color', content: '#0d0b14' } },
			],
			sidebar: [
				{
					label: 'Start here',
					items: [
						{ label: 'Overview', slug: 'docs' },
						{ label: 'Install', slug: 'docs/install' },
					],
				},
				{
					label: 'Services',
					items: [
						{ label: 'Spotify', slug: 'docs/spotify' },
						{ label: 'YouTube Music', slug: 'docs/youtube-music' },
					],
				},
				{
					label: 'Using cloofy',
					items: [
						{ label: 'Keys and mouse', slug: 'docs/keys' },
						{ label: 'Settings', slug: 'docs/settings' },
						{ label: 'Command line', slug: 'docs/cli' },
						{ label: 'Updates', slug: 'docs/updates' },
					],
				},
				{
					label: 'Your system',
					items: [
						{ label: 'Omarchy', slug: 'docs/omarchy' },
						{ label: 'Terminals', slug: 'docs/terminals' },
					],
				},
				{
					label: 'Help',
					items: [
						{ label: 'FAQ', slug: 'docs/faq' },
						{ label: 'Troubleshooting', slug: 'docs/troubleshooting' },
						{ label: 'Changelog', link: `${repo}/blob/main/CHANGELOG.md`, attrs: { target: '_blank' } },
					],
				},
			],
		}),
	],
});
