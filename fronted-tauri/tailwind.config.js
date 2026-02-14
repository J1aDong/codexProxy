/** @type {import('tailwindcss').Config} */
export default {
  darkMode: 'class',
  content: [
    "./index.html",
    "./src/**/*.{vue,js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        'apple-blue': '#0071e3',
        'apple-gray': '#f5f5f7',
        'apple-text-primary': '#1d1d1f',
        'apple-text-secondary': '#86868b',
        'apple-success': '#34c759',
        'apple-danger': '#ff3b30',
        // Dark theme colors
        'dark-primary': '#0a0a0f',
        'dark-secondary': '#14141a',
        'dark-tertiary': '#1c1c24',
        'dark-border': '#25252d',
        'dark-border-hover': '#3a3a4a',
        'dark-text-primary': '#f0f0f5',
        'dark-text-secondary': '#8b8b9a',
        'dark-text-tertiary': '#6b6b7b',
        'accent-blue': '#3b82f6',
        'accent-green': '#10b981',
        'accent-red': '#ef4444',
        'accent-purple': '#8b5cf6',
      },
      fontFamily: {
        'sf': ['-apple-system', 'BlinkMacSystemFont', '"SF Pro Text"', 'system-ui', 'sans-serif'],
        'sf-mono': ['"SF Mono"', 'monospace'],
      },
    },
  },
  plugins: [],
}
