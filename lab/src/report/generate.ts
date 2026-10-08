// Decisions report generated from the registry and the computed verdicts only: no figure is typed by hand.
import type { Registry } from '../core/decisions.ts';

type VerdictLike = {
  decisionId: string; outcome: string; n: number; requiredN: number | null; option?: string; baseline?: string; metricId?: string;
  estimate: { ratio: number; ratioCi: number[]; interval: string; wilcoxon: { pValue: number } } | null;
  gates: { id: string; ok: boolean; evaluated?: boolean }[]; host: { required: string; observed: string | null; eligible: boolean }; inputs: { runs: string[] };
};

const outcomeLabel: Record<string, string> = {
  better: 'option meilleure', worse: 'option moins bonne', equivalent: 'équivalentes', trivial: 'écart réel mais sous le seuil',
  'non-inferior': 'non inférieure', inconclusive: 'non concluant', 'blocked-by-contract': 'bloqué par un contrat', 'insufficient-n': 'blocs insuffisants'
};
const figure = (value: number) => Number(value.toPrecision(3)).toString();

export function renderDecisionsReport(registry: Registry, verdicts: VerdictLike[]): string {
  const byId = new Map(verdicts.map(verdict => [verdict.decisionId, verdict]));
  const lines = ['# Décisions du lab', '', 'Rapport généré par `pnpm lab report` depuis `decisions/` et `verdicts/`. Les chiffres viennent des verdicts calculés ; aucune valeur n\'est saisie à la main.', '',
    '| Décision | Statut | Verdict | Estimation option / baseline | Blocs | Hôte | Gates |', '| --- | --- | --- | --- | --- | --- | --- |'];
  for (const decision of [...registry.decisions].sort((a, b) => a.id.localeCompare(b.id))) {
    const verdict = byId.get(decision.id);
    if (decision.kind === 'product') { lines.push(`| ${decision.id} | ${decision.status} | produit : ${decision.evidence.map(e => e.ref).join(', ') || 'aucune preuve'} | | | | |`); continue; }
    if (!verdict) { lines.push(`| ${decision.id} | ${decision.status} | pas encore calculé | | | requis ${decision.hostClass?.decision ?? '?'} | ${(decision.contractGates ?? []).join(', ')} |`); continue; }
    const estimate = verdict.estimate ? `${figure(verdict.estimate.ratio)} [${verdict.estimate.ratioCi.map(figure).join(' ; ')}] (${verdict.estimate.interval})` : '';
    const blocks = verdict.requiredN === null ? `${verdict.n}` : `${verdict.n} sur ${verdict.requiredN} requis`;
    const host = `${verdict.host.observed ?? 'aucun'} / requis ${verdict.host.required}${verdict.host.eligible ? '' : ' (non éligible)'}`;
    const gates = verdict.gates.map(gate => `${gate.id} ${gate.evaluated === false ? 'non évaluée' : gate.ok ? 'ok' : 'échec'}`).join(', ');
    lines.push(`| ${decision.id} | ${decision.status} | ${outcomeLabel[verdict.outcome] ?? verdict.outcome} | ${estimate} | ${blocks} | ${host} | ${gates} |`);
  }
  lines.push('', 'Un verdict ne sert une décision que si l\'hôte observé atteint la classe requise et que toutes les gates de contrat passent.', '');
  return lines.join('\n');
}
