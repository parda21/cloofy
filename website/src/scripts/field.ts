// The hero's background: a field of pixels that plays like a spectrum analyzer. Bars rise
// from the bottom on a beat, their peaks fall back slowly, single pixels twinkle above, and
// the field lights up around the pointer. With reduced motion it holds still.

const PINK = [255, 143, 177];
const VIOLET = [139, 92, 246];
const INDIGO = [99, 102, 241];

/** A stable pseudo-random number in 0..1 for a cell, for dithering that doesn't flicker. */
function hash(x: number, y: number): number {
	let h = (x * 374761393 + y * 668265263) | 0;
	h = (h ^ (h >>> 13)) * 1274126177;
	return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
}

function mix(a: number[], b: number[], t: number): number[] {
	return a.map((v, i) => Math.round(v + (b[i] - v) * t));
}

/** The color of column `x` of `cols`: pink on the left, violet in the middle, indigo on the right. */
function color(x: number, cols: number): number[] {
	const t = cols > 1 ? x / (cols - 1) : 0;
	return t < 0.5 ? mix(PINK, VIOLET, t * 2) : mix(VIOLET, INDIGO, (t - 0.5) * 2);
}

interface Spark {
	x: number;
	y: number;
	phase: number;
	speed: number;
}

export function startField(canvas: HTMLCanvasElement): void {
	const ctx = canvas.getContext('2d');
	if (!ctx) return;
	const still = window.matchMedia('(prefers-reduced-motion: reduce)');
	let cell = 14;
	let cols = 0;
	let rows = 0;
	let dpr = 1;
	let peaks: number[] = [];
	let sparks: Spark[] = [];
	let pointer: { x: number; y: number } | null = null;
	let visible = true;
	let last = 0;
	let frame = 0;

	function resize(): void {
		const rect = canvas.getBoundingClientRect();
		dpr = Math.min(window.devicePixelRatio || 1, 2);
		cell = rect.width < 640 ? 10 : 14;
		canvas.width = Math.round(rect.width * dpr);
		canvas.height = Math.round(rect.height * dpr);
		cols = Math.ceil(rect.width / cell);
		rows = Math.ceil(rect.height / cell);
		peaks = Array.from({ length: cols }, () => 0);
		sparks = Array.from({ length: Math.round(cols * rows * 0.012) }, () => spawn(true));
	}

	function spawn(anywhere: boolean): Spark {
		return {
			x: Math.floor(Math.random() * cols),
			// Sparks keep to the upper two thirds, where the bars don't reach.
			y: Math.floor(Math.random() * rows * 0.7),
			phase: anywhere ? Math.random() * Math.PI : 0,
			speed: 0.6 + Math.random() * 1.4,
		};
	}

	/** How far up column `x` reaches at time `t`, in rows. */
	function level(x: number, t: number): number {
		const u = x / Math.max(cols, 1);
		// Bass on the left moves slower and reaches higher, as on a real analyzer.
		const bass = 1 - u * 0.55;
		const slow = 0.5 + 0.5 * Math.sin(t * (0.9 + u * 0.8) + x * 0.23);
		const fast = 0.5 + 0.5 * Math.sin(t * (2.3 + u * 3.1) - x * 0.61);
		const swell = 0.5 + 0.5 * Math.sin(t * 0.31 + u * 4.0);
		const beat = Math.exp(-((t % 0.5) / 0.09)) * 0.22 * bass;
		const v = 0.08 + bass * (0.42 * slow * swell + 0.22 * fast) + beat;
		return Math.max(0, v) * rows * 0.42;
	}

	function draw(t: number, dt: number): void {
		const w = canvas.width;
		const h = canvas.height;
		ctx!.clearRect(0, 0, w, h);
		const size = (cell - 3) * dpr;
		const put = (x: number, y: number, rgb: number[], alpha: number): void => {
			if (alpha <= 0.02) return;
			ctx!.fillStyle = `rgba(${rgb[0]},${rgb[1]},${rgb[2]},${Math.min(alpha, 1)})`;
			ctx!.fillRect(x * cell * dpr, h - (y + 1) * cell * dpr, size, size);
		};
		// Calmer behind the words in the middle.
		const center = (x: number, y: number): number => {
			const dx = (x - cols / 2) / (cols / 2);
			const dy = (y - rows * 0.55) / (rows / 2);
			return Math.min(1, 0.25 + Math.hypot(dx * 1.1, dy * 1.6));
		};
		const near = (x: number, y: number): number => {
			if (!pointer) return 0;
			const d = Math.hypot(x - pointer.x, rows - 1 - y - pointer.y);
			return d < 7 ? (1 - d / 7) * 0.55 : 0;
		};

		for (let x = 0; x < cols; x++) {
			const rgb = color(x, cols);
			const height = level(x, t);
			peaks[x] = Math.max(height, peaks[x] - dt * rows * 0.12);
			const top = Math.floor(height);
			for (let y = 0; y <= top; y++) {
				const along = height > 0 ? y / height : 0;
				// Thinning out toward the top of the bar, in a dither that holds still.
				if (along > 0.45 && hash(x, y) < (along - 0.45) * 1.4) continue;
				put(x, y, rgb, (0.75 - along * 0.55) * center(x, y) + near(x, y));
			}
			const peak = Math.floor(peaks[x]) + 1;
			if (peak > top + 1) put(x, peak, mix(rgb, [255, 255, 255], 0.35), 0.6 * center(x, peak));
		}

		for (const spark of sparks) {
			spark.phase += dt * spark.speed;
			if (spark.phase > Math.PI) Object.assign(spark, spawn(false));
			const y = rows - 1 - spark.y;
			put(spark.x, y, color(spark.x, cols), Math.sin(spark.phase) * 0.45 * center(spark.x, y) + near(spark.x, y));
		}

		// The pointer lights the field around it, also where nothing plays.
		if (pointer) {
			const px = Math.round(pointer.x);
			const py = Math.round(rows - 1 - pointer.y);
			for (let x = px - 6; x <= px + 6; x++) {
				for (let y = py - 6; y <= py + 6; y++) {
					if (x < 0 || y < 0 || x >= cols || y >= rows || hash(x, y) > 0.35) continue;
					put(x, y, color(x, cols), near(x, y) * 0.8);
				}
			}
		}
	}

	function loop(now: number): void {
		frame = requestAnimationFrame(loop);
		// About 30 frames a second is plenty for pixels this size.
		if (now - last < 33) return;
		const dt = last ? Math.min((now - last) / 1000, 0.1) : 0.033;
		last = now;
		draw(now / 1000, dt);
	}

	function run(): void {
		cancelAnimationFrame(frame);
		last = 0;
		if (still.matches || !visible) {
			draw(12.3, 0);
			return;
		}
		frame = requestAnimationFrame(loop);
	}

	resize();
	run();
	new ResizeObserver(() => {
		resize();
		run();
	}).observe(canvas);
	new IntersectionObserver(([entry]) => {
		visible = entry.isIntersecting;
		run();
	}).observe(canvas);
	still.addEventListener('change', run);
	document.addEventListener('visibilitychange', () => {
		visible = !document.hidden;
		run();
	});
	canvas.parentElement?.addEventListener('pointermove', (event) => {
		const rect = canvas.getBoundingClientRect();
		pointer = { x: (event.clientX - rect.left) / cell, y: (event.clientY - rect.top) / cell };
	});
	canvas.parentElement?.addEventListener('pointerleave', () => {
		pointer = null;
	});
}
