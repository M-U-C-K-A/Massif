// The FAQ: questions and their answers in Markdown, from facts of the README,
// the guide and the release notes.

export function faq({ url, REPO, config }) {
  return [
    ['Is Massif free?',
      `Yes. Massif is free and open source, under the GPL-3.0-or-later like WorldPainter. Its code is on [GitHub](${REPO}).`],
    ['Which Minecraft does it make maps for?',
      'Minecraft **Java Edition**. Massif exports for every version from 1.7.10 to 26.3, and for whatever version is installed, each in the chunk format it reads, with worlds up to 2,032 blocks high.'],
    ['Do I need Minecraft installed?',
      "Massif reads the real blocks, their textures and colours, from the game installed on your computer (official launcher, CurseForge, Prism). Nothing from the game is included with Massif."],
    ['Can I open my WorldPainter worlds?',
      `Yes: \`.world\` files are read directly, without Java, a 100 MB world in about a second, and your WorldPainter brushes folder is read as it is. Massif is still young, and some of WorldPainter's features, layers in particular, are still on the way.`],
    ['How large can a map be?',
      '60,000 × 50,000 blocks, generated in a minute and edited at once. A 16k × 16k map takes a few hundred megabytes of memory.'],
    ['Why does my computer warn me about Massif?',
      `The builds are not signed by Apple or Microsoft, so macOS and Windows ask before opening them the first time. [Download](${url('/download/')}) says what to click on each system.`],
    ['Does Massif send anything about me?',
      'Once a day at most, Massif asks GitHub which version is the latest, and sends nothing about you or your maps; **Settings ▸ Updates** turns it off. It never downloads or installs anything itself. Its start-up log stays on your computer.'],
    ['Massif does not start. What can I do?',
      `Look at [If Massif does not start](${url('/download/#if-massif-does-not-start')}): where the log is, and how to choose another graphics API. Then [open an issue](${REPO}/issues) with the log.`],
    ['Where do I report a bug or ask for a feature?',
      `In **#bug-reports** on [Massif's Discord](${config.discord}), or on [GitHub's issues](${REPO}/issues). **Help ▸ Report a Problem…** in Massif makes one file to send. For a problem at start-up, add \`startup.log\` and \`startup.previous.log\`.`],
    ['Where can I ask a question?',
      `On [Massif's Discord](${config.discord}), the community's server: questions, maps to show, ideas. **Help ▸ Join the Discord…** opens it from Massif.`],
  ];
}
