// Responsive layout helpers.
//
// Provides a reactive `isMobile` flag (and the media query it tracks) so the
// heavily inline-styled app can switch to a compact, stacked, drawer-based
// layout on narrow viewports without a CSS-file rewrite of every inline style.
//
// Breakpoint: 768px — below this we treat the viewport as a phone/tablet
// ("mobile") layout: full-bleed map with a slide-over panel drawer, wrapping
// header controls, and 100dvh height (accounting for the mobile browser chrome).

import { useEffect, useState } from "react";

export const MOBILE_QUERY = "(max-width: 767px)";

function snapshot(): boolean {
	if (typeof window === "undefined" || !window.matchMedia) return false;
	return window.matchMedia(MOBILE_QUERY).matches;
}

/** Reactive "is the viewport mobile-sized?" (≤767px). */
export function useIsMobile(): boolean {
	const [isMobile, setIsMobile] = useState<boolean>(snapshot);

	useEffect(() => {
		// SSR / very old browsers have no matchMedia; fall back to current size.
		if (!window.matchMedia) return;
		const mql = window.matchMedia(MOBILE_QUERY);
		const onChange = (e: MediaQueryListEvent) => setIsMobile(e.matches);
		// Set the initial value from the live media query (matches `snapshot`).
		setIsMobile(mql.matches);
		mql.addEventListener("change", onChange);
		return () => mql.removeEventListener("change", onChange);
	}, []);

	return isMobile;
}