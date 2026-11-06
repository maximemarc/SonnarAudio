// Conventional Commits — enforced by the commit-msg hook (see .husky/) and
// by CI on every pull request. Rules and examples: CONTRIBUTING.md.
export default {
  extends: ["@commitlint/config-conventional"],
  // Les commits de Dependabot ont un titre conforme (« chore(deps): … »)
  // mais un corps généré (liens, YAML) aux lignes de plus de 100
  // caractères : sans ça, chacune de ses PR échoue au job commitlint.
  // Repérés par la signature que Dependabot ajoute toujours.
  ignores: [(message) => message.includes("Signed-off-by: dependabot[bot]")],
};
