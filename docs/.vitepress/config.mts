import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Archivis',
  description: 'Self-hosted ebook collection manager',
  base: '/archivis/',
  cleanUrls: true,
  lastUpdated: true,

  themeConfig: {
    nav: [
      { text: 'Docs', link: '/guide/introduction' },
    ],

    sidebar: [
      {
        text: 'User Guide',
        items: [
          { text: 'Introduction', link: '/guide/introduction' },
          { text: 'Quick Start', link: '/guide/quick-start' },
          { text: 'Authentication', link: '/guide/authentication' },
          { text: 'Deployment', link: '/guide/deployment' },
        ],
      },
      {
        text: 'Development',
        items: [
          { text: 'Architecture', link: '/dev/architecture' },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/geekifier/archivis' },
    ],

    search: {
      provider: 'local',
    },
  },
})
