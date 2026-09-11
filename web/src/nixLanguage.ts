import nixGrammarUrl from "./nix/nix.tmLanguage.json?url";
import nixLanguageConfigurationUrl from "./nix/nix-language-configuration.json?url";

export { nixGrammarUrl, nixLanguageConfigurationUrl };

export const nixExtension = {
  name: "nix-grammar",
  publisher: "supervisord-now",
  version: "1.0.0",
  engines: {
    vscode: "*",
  },
  contributes: {
    languages: [
      {
        id: "nix",
        aliases: ["Nix", "nix"],
        extensions: [".nix"],
        configuration: "./nix-language-configuration.json",
      },
    ],
    grammars: [
      {
        language: "nix",
        scopeName: "source.nix",
        path: "./nix.tmLanguage.json",
      },
    ],
  },
};
