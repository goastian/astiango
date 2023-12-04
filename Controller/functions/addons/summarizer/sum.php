<?php
require_once $sumPath.'Controller/functions/addons/summarizer/TextRankFacade.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/Graph.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/Parser.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/Score.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/StopWords/StopWordsAbstract.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/StopWords/English.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/Summarize.php';
require_once $sumPath.'Controller/functions/addons/summarizer/Tool/Text.php';

use PhpScience\TextRank\TextRankFacade;

function summarizeText($text, $numSentences) {
    $facade = new TextRankFacade();
    $summarizedText = $facade->summarizeTextFreely($text, 10, $numSentences, 1);
    return $summarizedText;
  }