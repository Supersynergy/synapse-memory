# LightGBM LambdaMART Reranker
Expected NDCG@10 improvement: +8-15% over linear fusion.
Train: `python lightgbm_rerank.py train --db ~/.synapse/brain.db --out ~/.synapse/rerank.lgb`
Score: `python lightgbm_rerank.py score --model ~/.synapse/rerank.lgb --candidates '[{"vec_score":0.9,...}]'`
Log clicks to `synapse_rerank_log` table to improve over time.
