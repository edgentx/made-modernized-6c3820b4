interface PlaceholderProps {
  title: string
  blurb: string
}

/**
 * Scaffold placeholder for a not-yet-implemented view. Later stories replace
 * each view's body; S-78 only establishes the route + shell.
 */
export default function Placeholder({ title, blurb }: PlaceholderProps) {
  return (
    <section className="view">
      <h1 className="view__title">{title}</h1>
      <p className="view__blurb">{blurb}</p>
      <p className="view__stub">Placeholder — implemented in a later story.</p>
    </section>
  )
}
