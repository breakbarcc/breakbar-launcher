// The languages of the site. English lives at the root, the others below their prefix.
// `shots` is the folder of the screenshots shown on that language's pages.
export default [
  { code: "en", prefix: "", label: "English", short: "EN", shots: "/assets/shots/en" },
  // TODO: German screenshots (BREAKBAR_PREVIEW_LANG=de); until they exist the English ones are shown.
  { code: "de", prefix: "/de", label: "Deutsch", short: "DE", shots: "/assets/shots/en" },
];
