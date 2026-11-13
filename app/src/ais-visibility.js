export function aisVisibleInView(visibilityByView, view, legacyVisible) {
  const saved = visibilityByView?.[view];
  if (typeof saved === "boolean") return saved;
  return view === "plan" ? false : !!legacyVisible;
}
