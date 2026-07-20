import org.languagetool.JLanguageTool;
import org.languagetool.Language;
import org.languagetool.Languages;
import org.languagetool.AnalyzedSentence;
import org.languagetool.AnalyzedTokenReadings;
import org.languagetool.AnalyzedToken;
import org.languagetool.tagging.disambiguation.rules.XmlRuleDisambiguator;

import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.List;

/**
 * Dumps the post-disambiguation analyzed sentence, isolating the XML rule
 * disambiguator (getRawAnalyzedSentence = tagging only, then XmlRuleDisambiguator)
 * so it matches exactly what crates/matcher's disambig layer targets. The English
 * multiword chunker (multiwords.txt) is deliberately NOT applied here.
 *
 * Usage: AnalyzedOracle <raw|dis> <probes.txt>
 * Each non-empty line of probes is one input text (\n / \t escapes honored).
 * raw = tagging only (getRawAnalyzedSentence); dis = + XmlRuleDisambiguator.
 */
public class AnalyzedOracle {
    public static void main(String[] args) throws Exception {
        boolean disambiguate = !args[0].equals("raw");
        Language lang = Languages.getLanguageForShortCode("en-US");
        JLanguageTool lt = new JLanguageTool(lang);
        XmlRuleDisambiguator dis = new XmlRuleDisambiguator(lang);

        List<String> lines = Files.readAllLines(Paths.get(args[1]));
        StringBuilder out = new StringBuilder();
        for (String raw : lines) {
            if (raw.isEmpty()) continue;
            String text = raw.replace("\\n", "\n").replace("\\t", "\t");
            out.append("===|").append(raw).append('\n');
            List<String> sentences = lt.sentenceTokenize(text);
            for (String s : sentences) {
                AnalyzedSentence rawAn = lt.getRawAnalyzedSentence(s);
                AnalyzedSentence an = disambiguate ? dis.disambiguate(rawAn) : rawAn;
                out.append("SENT|").append(visible(s)).append('\n');
                for (AnalyzedTokenReadings tok : an.getTokensWithoutWhitespace()) {
                    out.append("TOK|").append(visible(tok.getToken())).append('|');
                    AnalyzedToken[] rs = tok.getReadings().toArray(new AnalyzedToken[0]);
                    for (int i = 0; i < rs.length; i++) {
                        if (i > 0) out.append('#');
                        out.append(reading(rs[i]));
                    }
                    out.append('\n');
                }
            }
        }
        System.out.print(out);
    }

    // lemma/POS, with explicit sentinels for null so Rust can reproduce exactly.
    static String reading(AnalyzedToken r) {
        String lemma = r.getLemma();
        String pos = r.getPOSTag();
        return (lemma == null ? "∅" : visible(lemma)) + "/" + (pos == null ? "∅" : pos);
    }

    static String visible(String s) {
        return s.replace("\n", "\\n").replace("\t", "\\t");
    }
}
