// Product code whose `threshold` is an IntersectionObserver's, no tolerance.
export function watchVisible(element: Element, onVisible: () => void): IntersectionObserver {
  const observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        onVisible();
      }
    },
    { threshold: 0.5 },
  );
  observer.observe(element);
  return observer;
}
