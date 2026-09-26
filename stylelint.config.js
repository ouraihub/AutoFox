const semanticColor = /^(?:var\(|currentColor$|inherit$|transparent$|none$|unset$|initial$|revert)/i;

/** @type {import('stylelint').Config} */
export default {
  overrides: [{ files: ['**/*.svelte'], customSyntax: 'postcss-html' }],
  rules: {
    'block-no-empty': true,
    'declaration-block-no-duplicate-properties': true,
    'declaration-property-value-allowed-list': {
      color: [semanticColor],
      'background-color': [semanticColor],
      'border-color': [semanticColor],
      'outline-color': [semanticColor],
      'accent-color': [semanticColor],
      fill: [semanticColor],
      stroke: [semanticColor],
    },
  },
  ignoreFiles: ['node_modules/**', 'build/**', '.svelte-kit/**', 'src/lib/components/ui/**'],
};
