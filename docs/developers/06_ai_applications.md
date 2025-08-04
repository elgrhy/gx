# 🤖 Building AI Applications with GX

## Overview

In this tutorial, we'll build AI applications using GX, including machine learning models, natural language processing, computer vision, and intelligent automation. We'll learn how to create intelligent systems with GX's brain-first approach.

## Architecture Overview

Our AI applications will include:
- **ML Model Manager**: Handles model training, deployment, and versioning
- **NLP Processor**: Natural language processing and text analysis
- **Computer Vision**: Image and video processing capabilities
- **Recommendation Engine**: Intelligent content and product recommendations
- **Predictive Analytics**: Data analysis and forecasting
- **AI Orchestrator**: Coordinates multiple AI services

## Step 1: Machine Learning Model Manager

```gx
helper "ml_model_manager" {
  can_do: ["model_training", "model_deployment", "version_control"]
  
  remember {
    models = {}
    training_jobs = {}
    model_registry = {}
    performance_metrics = {}
  }

  brain {
    plan {
      plan = { action: "manage_models" }
    }

    execute {
      if plan.action == "manage_models" {
        // Monitor training jobs
        monitor_training_jobs()
        
        // Deploy ready models
        deploy_ready_models()
        
        // Update model performance
        update_model_performance()
        
        // Clean up old models
        cleanup_old_models()
      }
    }
  }

  recipe "train_model" {
    needs: model_config, training_data
    gives: training_job_id
    
    brain {
      plan {
        plan = { action: "train" }
      }
      
      execute {
        if plan.action == "train" {
          // Generate training job ID
          training_job_id = generate_job_id()
          
          // Create training job
          training_job = {
            id: training_job_id,
            model_type: model_config.type,
            hyperparameters: model_config.hyperparameters,
            training_data: training_data,
            status: "queued",
            created_at: get_timestamp(),
            progress: 0
          }
          
          memory.training_jobs[training_job_id] = training_job
          
          // Start training process
          start_training_process(training_job_id)
        }
      }
    }
  }

  recipe "start_training_process" {
    needs: job_id
    gives: training_status
    
    brain {
      plan {
        plan = { action: "start" }
      }
      
      execute {
        if plan.action == "start" {
          job = memory.training_jobs[job_id]
          
          // Update job status
          job.status = "training"
          job.started_at = get_timestamp()
          
          // Initialize model based on type
          if job.model_type === "classification" {
            model = initialize_classification_model(job.hyperparameters)
          } else if job.model_type === "regression" {
            model = initialize_regression_model(job.hyperparameters)
          } else if job.model_type === "clustering" {
            model = initialize_clustering_model(job.hyperparameters)
          } else if job.model_type === "neural_network" {
            model = initialize_neural_network(job.hyperparameters)
          }
          
          // Train model
          training_result = train_model_with_data(model, job.training_data)
          
          // Save trained model
          model_id = save_trained_model(training_result.model, job)
          
          // Update job status
          job.status = "completed"
          job.completed_at = get_timestamp()
          job.model_id = model_id
          job.performance = training_result.performance
          
          training_status = {
            job_id: job_id,
            status: "completed",
            model_id: model_id,
            performance: training_result.performance
          }
        }
      }
    }
  }

  recipe "deploy_model" {
    needs: model_id, deployment_config
    gives: deployment_status
    
    brain {
      plan {
        plan = { action: "deploy" }
      }
      
      execute {
        if plan.action == "deploy" {
          model = memory.models[model_id]
          
          if model && model.status === "trained" {
            // Create deployment
            deployment = {
              id: generate_deployment_id(),
              model_id: model_id,
              version: model.version,
              environment: deployment_config.environment,
              resources: deployment_config.resources,
              status: "deploying",
              created_at: get_timestamp()
            }
            
            // Deploy model to target environment
            deployment_result = deploy_model_to_environment(model, deployment_config)
            
            if deployment_result.success {
              deployment.status = "active"
              deployment.endpoint = deployment_result.endpoint
              
              // Update model status
              model.status = "deployed"
              model.active_deployment = deployment.id
              
              deployment_status = {
                success: true,
                deployment_id: deployment.id,
                endpoint: deployment_result.endpoint
              }
            } else {
              deployment.status = "failed"
              deployment.error = deployment_result.error
              
              deployment_status = {
                success: false,
                error: deployment_result.error
              }
            }
          } else {
            deployment_status = {
              success: false,
              error: "Model not ready for deployment"
            }
          }
        }
      }
    }
  }

  recipe "predict" {
    needs: model_id, input_data
    gives: prediction
    
    brain {
      plan {
        plan = { action: "predict" }
      }
      
      execute {
        if plan.action == "predict" {
          model = memory.models[model_id]
          
          if model && model.status === "deployed" {
            // Load model
            loaded_model = load_model(model.model_path)
            
            // Preprocess input data
            processed_data = preprocess_input(input_data, model.preprocessing_config)
            
            // Make prediction
            raw_prediction = loaded_model.predict(processed_data)
            
            // Postprocess prediction
            prediction = postprocess_prediction(raw_prediction, model.postprocessing_config)
            
            // Log prediction
            log_prediction(model_id, input_data, prediction)
          } else {
            prediction = {
              error: "Model not available for prediction"
            }
          }
        }
      }
    }
  }
}
```

## Step 2: Natural Language Processing

```gx
helper "nlp_processor" {
  can_do: ["text_processing", "language_detection", "sentiment_analysis"]
  
  remember {
    nlp_models = {}
    language_models = {}
    sentiment_models = {}
    text_processors = {}
  }

  brain {
    plan {
      plan = { action: "process_nlp" }
    }

    execute {
      if plan.action == "process_nlp" {
        // Process text analysis requests
        process_text_requests()
        
        // Update language models
        update_language_models()
        
        // Optimize sentiment analysis
        optimize_sentiment_analysis()
      }
    }
  }

  recipe "analyze_text" {
    needs: text, analysis_type
    gives: analysis_result
    
    brain {
      plan {
        plan = { action: "analyze" }
      }
      
      execute {
        if plan.action == "analyze" {
          analysis_result = {
            text: text,
            analysis_type: analysis_type,
            results: {}
          }
          
          // Language detection
          if analysis_type.includes("language") {
            language = detect_language(text)
            analysis_result.results.language = language
          }
          
          // Sentiment analysis
          if analysis_type.includes("sentiment") {
            sentiment = analyze_sentiment(text)
            analysis_result.results.sentiment = sentiment
          }
          
          // Entity extraction
          if analysis_type.includes("entities") {
            entities = extract_entities(text)
            analysis_result.results.entities = entities
          }
          
          // Keyword extraction
          if analysis_type.includes("keywords") {
            keywords = extract_keywords(text)
            analysis_result.results.keywords = keywords
          }
          
          // Text classification
          if analysis_type.includes("classification") {
            classification = classify_text(text)
            analysis_result.results.classification = classification
          }
        }
      }
    }
  }

  recipe "detect_language" {
    needs: text
    gives: language_info
    
    brain {
      plan {
        plan = { action: "detect" }
      }
      
      execute {
        if plan.action == "detect" {
          // Use language detection model
          model = memory.language_models.detector
          
          if model {
            // Preprocess text
            processed_text = preprocess_text_for_language_detection(text)
            
            // Detect language
            detection_result = model.predict(processed_text)
            
            language_info = {
              language: detection_result.language,
              confidence: detection_result.confidence,
              alternatives: detection_result.alternatives
            }
          } else {
            // Fallback to rule-based detection
            language_info = rule_based_language_detection(text)
          }
        }
      }
    }
  }

  recipe "analyze_sentiment" {
    needs: text
    gives: sentiment_result
    
    brain {
      plan {
        plan = { action: "analyze" }
      }
      
      execute {
        if plan.action == "analyze" {
          // Use sentiment analysis model
          model = memory.sentiment_models.analyzer
          
          if model {
            // Preprocess text
            processed_text = preprocess_text_for_sentiment(text)
            
            // Analyze sentiment
            sentiment_prediction = model.predict(processed_text)
            
            sentiment_result = {
              sentiment: sentiment_prediction.sentiment, // positive, negative, neutral
              score: sentiment_prediction.score,
              confidence: sentiment_prediction.confidence,
              aspects: extract_sentiment_aspects(text)
            }
          } else {
            // Fallback to lexicon-based analysis
            sentiment_result = lexicon_based_sentiment_analysis(text)
          }
        }
      }
    }
  }

  recipe "extract_entities" {
    needs: text
    gives: entities
    
    brain {
      plan {
        plan = { action: "extract" }
      }
      
      execute {
        if plan.action == "extract" {
          // Use NER model
          ner_model = memory.nlp_models.ner
          
          if ner_model {
            // Extract named entities
            entities = ner_model.extract_entities(text)
          } else {
            // Fallback to rule-based extraction
            entities = rule_based_entity_extraction(text)
          }
          
          // Categorize entities
          categorized_entities = {
            persons: [],
            organizations: [],
            locations: [],
            dates: [],
            numbers: []
          }
          
          for each entity in entities {
            if entity.type === "PERSON" {
              categorized_entities.persons.push(entity)
            } else if entity.type === "ORG" {
              categorized_entities.organizations.push(entity)
            } else if entity.type === "LOC" {
              categorized_entities.locations.push(entity)
            } else if entity.type === "DATE" {
              categorized_entities.dates.push(entity)
            } else if entity.type === "NUMBER" {
              categorized_entities.numbers.push(entity)
            }
          }
          
          entities = categorized_entities
        }
      }
    }
  }

  recipe "generate_text" {
    needs: prompt, generation_config
    gives: generated_text
    
    brain {
      plan {
        plan = { action: "generate" }
      }
      
      execute {
        if plan.action == "generate" {
          // Use text generation model
          model = memory.nlp_models.generator
          
          if model {
            // Preprocess prompt
            processed_prompt = preprocess_prompt(prompt)
            
            // Generate text
            generation_result = model.generate(processed_prompt, generation_config)
            
            generated_text = {
              text: generation_result.text,
              confidence: generation_result.confidence,
              tokens_used: generation_result.tokens_used,
              generation_time: generation_result.generation_time
            }
          } else {
            generated_text = {
              error: "Text generation model not available"
            }
          }
        }
      }
    }
  }
}
```

## Step 3: Computer Vision System

```gx
helper "computer_vision" {
  can_do: ["image_processing", "object_detection", "image_classification"]
  
  remember {
    vision_models = {}
    image_processors = {}
    detection_results = {}
  }

  brain {
    plan {
      plan = { action: "process_vision" }
    }

    execute {
      if plan.action == "process_vision" {
        // Process image analysis requests
        process_image_requests()
        
        // Update vision models
        update_vision_models()
        
        // Optimize image processing
        optimize_image_processing()
      }
    }
  }

  recipe "analyze_image" {
    needs: image_data, analysis_type
    gives: analysis_result
    
    brain {
      plan {
        plan = { action: "analyze" }
      }
      
      execute {
        if plan.action == "analyze" {
          analysis_result = {
            image_id: generate_image_id(),
            analysis_type: analysis_type,
            results: {}
          }
          
          // Object detection
          if analysis_type.includes("objects") {
            objects = detect_objects(image_data)
            analysis_result.results.objects = objects
          }
          
          // Image classification
          if analysis_type.includes("classification") {
            classification = classify_image(image_data)
            analysis_result.results.classification = classification
          }
          
          // Face detection
          if analysis_type.includes("faces") {
            faces = detect_faces(image_data)
            analysis_result.results.faces = faces
          }
          
          // Text recognition (OCR)
          if analysis_type.includes("text") {
            text = extract_text_from_image(image_data)
            analysis_result.results.text = text
          }
          
          // Image segmentation
          if analysis_type.includes("segmentation") {
            segments = segment_image(image_data)
            analysis_result.results.segments = segments
          }
        }
      }
    }
  }

  recipe "detect_objects" {
    needs: image_data
    gives: detected_objects
    
    brain {
      plan {
        plan = { action: "detect" }
      }
      
      execute {
        if plan.action == "detect" {
          // Use object detection model
          model = memory.vision_models.object_detector
          
          if model {
            // Preprocess image
            processed_image = preprocess_image_for_detection(image_data)
            
            // Detect objects
            detection_result = model.detect(processed_image)
            
            detected_objects = []
            
            for each detection in detection_result.detections {
              object_info = {
                label: detection.label,
                confidence: detection.confidence,
                bounding_box: detection.bbox,
                class_id: detection.class_id
              }
              
              detected_objects.push(object_info)
            }
          } else {
            detected_objects = {
              error: "Object detection model not available"
            }
          }
        }
      }
    }
  }

  recipe "classify_image" {
    needs: image_data
    gives: classification_result
    
    brain {
      plan {
        plan = { action: "classify" }
      }
      
      execute {
        if plan.action == "classify" {
          // Use image classification model
          model = memory.vision_models.classifier
          
          if model {
            // Preprocess image
            processed_image = preprocess_image_for_classification(image_data)
            
            // Classify image
            classification = model.classify(processed_image)
            
            classification_result = {
              top_class: classification.top_class,
              confidence: classification.confidence,
              all_classes: classification.all_classes
            }
          } else {
            classification_result = {
              error: "Image classification model not available"
            }
          }
        }
      }
    }
  }

  recipe "extract_text_from_image" {
    needs: image_data
    gives: extracted_text
    
    brain {
      plan {
        plan = { action: "extract" }
      }
      
      execute {
        if plan.action == "extract" {
          // Use OCR model
          ocr_model = memory.vision_models.ocr
          
          if ocr_model {
            // Preprocess image for OCR
            processed_image = preprocess_image_for_ocr(image_data)
            
            // Extract text
            ocr_result = ocr_model.extract_text(processed_image)
            
            extracted_text = {
              text: ocr_result.text,
              confidence: ocr_result.confidence,
              bounding_boxes: ocr_result.bboxes,
              language: ocr_result.language
            }
          } else {
            extracted_text = {
              error: "OCR model not available"
            }
          }
        }
      }
    }
  }
}
```

## Step 4: Recommendation Engine

```gx
helper "recommendation_engine" {
  can_do: ["content_recommendation", "collaborative_filtering", "personalization"]
  
  remember {
    recommendation_models = {}
    user_profiles = {}
    item_profiles = {}
    interaction_history = {}
  }

  brain {
    plan {
      plan = { action: "generate_recommendations" }
    }

    execute {
      if plan.action == "generate_recommendations" {
        // Process recommendation requests
        process_recommendation_requests()
        
        // Update user profiles
        update_user_profiles()
        
        // Train recommendation models
        train_recommendation_models()
      }
    }
  }

  recipe "get_recommendations" {
    needs: user_id, item_type, limit
    gives: recommendations
    
    brain {
      plan {
        plan = { action: "recommend" }
      }
      
      execute {
        if plan.action == "recommend" {
          // Get user profile
          user_profile = memory.user_profiles[user_id] || {}
          
          // Get user interaction history
          user_history = memory.interaction_history[user_id] || []
          
          // Generate recommendations using different algorithms
          collaborative_recommendations = get_collaborative_recommendations(user_id, item_type, limit)
          content_based_recommendations = get_content_based_recommendations(user_profile, item_type, limit)
          hybrid_recommendations = get_hybrid_recommendations(user_id, item_type, limit)
          
          // Combine and rank recommendations
          all_recommendations = combine_recommendations([
            collaborative_recommendations,
            content_based_recommendations,
            hybrid_recommendations
          ])
          
          // Filter and rank
          filtered_recommendations = filter_recommendations(all_recommendations, user_history)
          ranked_recommendations = rank_recommendations(filtered_recommendations, user_profile)
          
          recommendations = {
            user_id: user_id,
            recommendations: ranked_recommendations.slice(0, limit),
            algorithm_used: "hybrid",
            confidence_scores: calculate_confidence_scores(ranked_recommendations)
          }
        }
      }
    }
  }

  recipe "get_collaborative_recommendations" {
    needs: user_id, item_type, limit
    gives: recommendations
    
    brain {
      plan {
        plan = { action: "collaborative" }
      }
      
      execute {
        if plan.action == "collaborative" {
          // Find similar users
          similar_users = find_similar_users(user_id)
          
          // Get items liked by similar users
          candidate_items = []
          
          for each similar_user in similar_users {
            user_items = get_user_interactions(similar_user.id)
            for each item in user_items {
              if item.type === item_type && item.rating > 3 {
                candidate_items.push({
                  item_id: item.item_id,
                  score: item.rating * similar_user.similarity
                })
              }
            }
          }
          
          // Aggregate and rank items
          item_scores = aggregate_item_scores(candidate_items)
          recommendations = rank_items_by_score(item_scores, limit)
        }
      }
    }
  }

  recipe "get_content_based_recommendations" {
    needs: user_profile, item_type, limit
    gives: recommendations
    
    brain {
      plan {
        plan = { action: "content_based" }
      }
      
      execute {
        if plan.action == "content_based" {
          // Get user preferences
          user_preferences = extract_user_preferences(user_profile)
          
          // Get candidate items
          candidate_items = get_items_by_type(item_type)
          
          // Calculate similarity scores
          recommendations = []
          
          for each item in candidate_items {
            item_features = extract_item_features(item)
            similarity_score = calculate_similarity(user_preferences, item_features)
            
            if similarity_score > 0.5 {
              recommendations.push({
                item_id: item.id,
                score: similarity_score,
                features: item_features
              })
            }
          }
          
          // Sort by similarity score
          recommendations.sort((a, b) => b.score - a.score)
          recommendations = recommendations.slice(0, limit)
        }
      }
    }
  }

  recipe "update_user_profile" {
    needs: user_id, interaction_data
    gives: updated_profile
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          // Get current profile
          current_profile = memory.user_profiles[user_id] || {
            id: user_id,
            preferences: {},
            interaction_count: 0,
            last_updated: get_timestamp()
          }
          
          // Update interaction history
          if !memory.interaction_history[user_id] {
            memory.interaction_history[user_id] = []
          }
          
          memory.interaction_history[user_id].push({
            item_id: interaction_data.item_id,
            item_type: interaction_data.item_type,
            interaction_type: interaction_data.type, // view, like, purchase, etc.
            rating: interaction_data.rating,
            timestamp: get_timestamp()
          })
          
          // Update preferences based on interaction
          updated_preferences = update_preferences_based_on_interaction(
            current_profile.preferences,
            interaction_data
          )
          
          // Update profile
          updated_profile = {
            id: user_id,
            preferences: updated_preferences,
            interaction_count: current_profile.interaction_count + 1,
            last_updated: get_timestamp()
          }
          
          memory.user_profiles[user_id] = updated_profile
        }
      }
    }
  }
}
```

## Step 5: Predictive Analytics

```gx
helper "predictive_analytics" {
  can_do: ["forecasting", "trend_analysis", "anomaly_detection"]
  
  remember {
    forecasting_models = {}
    trend_analyzers = {}
    anomaly_detectors = {}
    historical_data = {}
  }

  brain {
    plan {
      plan = { action: "analyze_predictions" }
    }

    execute {
      if plan.action == "analyze_predictions" {
        // Process forecasting requests
        process_forecasting_requests()
        
        // Update trend analysis
        update_trend_analysis()
        
        // Detect anomalies
        detect_anomalies()
      }
    }
  }

  recipe "forecast_timeseries" {
    needs: data, forecast_period, model_type
    gives: forecast_result
    
    brain {
      plan {
        plan = { action: "forecast" }
      }
      
      execute {
        if plan.action == "forecast" {
          // Preprocess time series data
          processed_data = preprocess_timeseries_data(data)
          
          // Select forecasting model
          if model_type === "arima" {
            model = memory.forecasting_models.arima
          } else if model_type === "prophet" {
            model = memory.forecasting_models.prophet
          } else if model_type === "lstm" {
            model = memory.forecasting_models.lstm
          } else {
            model = memory.forecasting_models.default
          }
          
          // Train model if needed
          if !model.is_trained {
            model = train_forecasting_model(model, processed_data)
          }
          
          // Generate forecast
          forecast = model.forecast(processed_data, forecast_period)
          
          forecast_result = {
            predictions: forecast.predictions,
            confidence_intervals: forecast.confidence_intervals,
            model_accuracy: forecast.accuracy,
            forecast_period: forecast_period
          }
        }
      }
    }
  }

  recipe "detect_anomalies" {
    needs: data, detection_method
    gives: anomalies
    
    brain {
      plan {
        plan = { action: "detect" }
      }
      
      execute {
        if plan.action == "detect" {
          // Select anomaly detection method
          if detection_method === "isolation_forest" {
            detector = memory.anomaly_detectors.isolation_forest
          } else if detection_method === "autoencoder" {
            detector = memory.anomaly_detectors.autoencoder
          } else if detection_method === "statistical" {
            detector = memory.anomaly_detectors.statistical
          } else {
            detector = memory.anomaly_detectors.default
          }
          
          // Detect anomalies
          detection_result = detector.detect_anomalies(data)
          
          anomalies = {
            anomalies: detection_result.anomalies,
            scores: detection_result.scores,
            threshold: detection_result.threshold,
            method: detection_method
          }
        }
      }
    }
  }

  recipe "analyze_trends" {
    needs: data, analysis_period
    gives: trend_analysis
    
    brain {
      plan {
        plan = { action: "analyze" }
      }
      
      execute {
        if plan.action == "analyze" {
          // Analyze trends in the data
          trend_analyzer = memory.trend_analyzers.default
          
          analysis_result = trend_analyzer.analyze_trends(data, analysis_period)
          
          trend_analysis = {
            overall_trend: analysis_result.overall_trend,
            seasonal_patterns: analysis_result.seasonal_patterns,
            trend_strength: analysis_result.trend_strength,
            breakpoints: analysis_result.breakpoints,
            recommendations: generate_trend_recommendations(analysis_result)
          }
        }
      }
    }
  }
}
```

## Running AI Applications

1. **Save the complete application** to a file:
   ```bash
   # Save all helpers to ai_app.gx
   # (Include all the helper code above)
   ```

2. **Run the application**:
   ```bash
   ./bin/gx ai_app.gx
   ```

3. **Expected output**:
   ```
   🧠 GX Language Runtime v0.1.0 (Self-Hosting)
   =============================================
   
     📝 Loading GX file: ai_app.gx
     📊 File size: 14200 bytes
   
     🚀 Executing GX Runtime: ai_app.gx
     🧠 Initializing cognitive runtime...
     📊 Found 5 helpers with 25 brain processes
     🧠 Brain cycle: Plan → Execute → Remember → Communicate
     AI Application initialized successfully!
     ML Model Manager: Active
     NLP Processor: Active
     Computer Vision: Active
     Recommendation Engine: Active
     Predictive Analytics: Active
     ✅ GX Runtime execution completed successfully!
   
   🎉 GX Runtime completed successfully!
   ```

## Advanced Features to Add

1. **Deep Learning**: Implement neural networks and deep learning models
2. **Reinforcement Learning**: Add RL algorithms for decision making
3. **Federated Learning**: Implement distributed model training
4. **AutoML**: Automated machine learning pipeline
5. **Model Explainability**: Add interpretability features
6. **Edge AI**: Deploy models to edge devices

## Practice Exercises

1. **Build a sentiment analysis system** for social media monitoring
2. **Create an image classification API** for product categorization
3. **Make a recommendation system** for e-commerce products
4. **Build a predictive maintenance system** for IoT devices
5. **Create a chatbot** with natural language understanding

## Next Steps

Now that you understand AI applications, you're ready to:
- [Build a ChatGPT Clone](07_chatgpt_clone.md)
- [Build a TikTok Clone](08_tiktok_clone.md)
- [Create a Social Media Platform](09_social_media_platform.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: [www.devjsx.com](http://www.devjsx.com/)**

**Ahmed Elgarhy** - Founder of DEVJSX, AI Software Architect and cognitive programming pioneer. 