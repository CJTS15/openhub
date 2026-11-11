// ESLint flat config for the GNOME Shell extension. No plugins, so CI runs
// it with `npx eslint@10` and nothing to install:
//   npx --yes eslint@10.13.0 -c extension/eslint.config.js extension
export default [
    {
        files: ['**/*.js'],
        languageOptions: {
            ecmaVersion: 2024,
            sourceType: 'module',
            globals: {
                // GJS and GNOME Shell globals.
                global: 'readonly',
                imports: 'readonly',
                log: 'readonly',
                logError: 'readonly',
                print: 'readonly',
                printerr: 'readonly',
                console: 'readonly',
                ARGV: 'readonly',
                TextDecoder: 'readonly',
                TextEncoder: 'readonly',
                setTimeout: 'readonly',
                clearTimeout: 'readonly',
                setInterval: 'readonly',
                clearInterval: 'readonly',
            },
        },
        rules: {
            'no-undef': 'error',
            'no-unused-vars': ['error', {argsIgnorePattern: '^_'}],
            'no-unreachable': 'error',
            'no-dupe-keys': 'error',
            'no-redeclare': 'error',
            'no-const-assign': 'error',
            'no-self-assign': 'error',
            'eqeqeq': ['error', 'smart'],
            'prefer-const': 'error',
            'no-var': 'error',
        },
    },
];
