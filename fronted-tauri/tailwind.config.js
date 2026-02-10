/** @type {import('tailwindcss').Config} */
export default {
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
      },
      fontFamily: {
        'sf': ['-apple-system', 'BlinkMacSystemFont', '"SF Pro Text"', 'system-ui', 'sans-serif'],
        'sf-mono': ['"SF Mono"', 'monospace'],
      },
    },
  },
  plugins: [],
}
