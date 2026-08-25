/**
 * Заглушка раздела, который ещё не сделан.
 *
 * Честная: называет фазу и что появится. Пустой экран без объяснения выглядит поломкой,
 * а «скоро будет» ничего не сообщает.
 */

export function ComingSoon({
  title,
  phase,
  what,
  fallback,
}: {
  title: string;
  phase: string;
  what: string;
  /** Чем пользоваться, пока раздела нет. */
  fallback?: string;
}) {
  return (
    <div className="coming-soon">
      <h1>{title}</h1>
      <p className="coming-soon__phase">{phase}</p>
      <p>{what}</p>
      {fallback && (
        <p className="coming-soon__fallback">
          <strong>Пока пользуйтесь прежним способом:</strong> {fallback}
        </p>
      )}
    </div>
  );
}
