import { type MouseEvent, useCallback, useEffect, useState } from "react";

// Navigation inside the app, without a reload: a browser that is not
// remembered keeps its device key in memory, and a reload would make it a
// new device. The bucket answers every path with index.html, so a deep link
// still lands here.

/** The path and the fragment of the current location. */
export interface Location {
  pathname: string;
  hash: string;
}

function current(): Location {
  return { pathname: window.location.pathname, hash: window.location.hash };
}

export function navigate(to: string, replace = false): void {
  if (replace) window.history.replaceState(null, "", to);
  else window.history.pushState(null, "", to);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

/** The current location, following `navigate` and the back button. */
export function useLocation(): Location {
  const [location, setLocation] = useState(current);
  useEffect(() => {
    const onChange = () => setLocation(current());
    window.addEventListener("popstate", onChange);
    return () => window.removeEventListener("popstate", onChange);
  }, []);
  return location;
}

/** The click handler of an in-app link: the same tab, no reload. */
export function useLinkClick(): (e: MouseEvent<HTMLAnchorElement>) => void {
  return useCallback((e: MouseEvent<HTMLAnchorElement>) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const href = e.currentTarget.getAttribute("href");
    if (!href || !href.startsWith("/") || href.startsWith("/.ory/")) return;
    e.preventDefault();
    navigate(href);
  }, []);
}
