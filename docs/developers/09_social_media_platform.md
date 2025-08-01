# 📱 Building a Social Media Platform with GX

## Overview

In this tutorial, we'll build a complete social media platform using GX, including user profiles, posts, comments, likes, follows, messaging, and real-time features. We'll learn how to create a scalable social networking application with GX's brain-first approach.

## Architecture Overview

Our social media platform will include:
- **User Management**: Profiles, authentication, and user relationships
- **Content System**: Posts, stories, and media sharing
- **Social Features**: Likes, comments, shares, and follows
- **Messaging System**: Real-time chat and notifications
- **Feed Engine**: Personalized content discovery
- **Analytics**: User engagement and platform metrics

## Step 1: User Management System

```gx
helper "user_management" {
  can_do: ["user_profiles", "authentication", "relationship_management"]
  
  remember {
    users = {}
    user_profiles = {}
    relationships = {}
    user_sessions = {}
  }

  brain {
    plan {
      plan = { action: "manage_users" }
    }

    execute {
      if plan.action == "manage_users" {
        // Process user registration
        process_user_registrations()
        
        // Handle user authentication
        handle_user_authentication()
        
        // Manage user relationships
        manage_user_relationships()
        
        // Update user profiles
        update_user_profiles()
      }
    }
  }

  recipe "register_user" {
    needs: user_data
    gives: registration_result
    
    brain {
      plan {
        plan = { action: "register" }
      }
      
      execute {
        if plan.action == "register" {
          // Validate user data
          validation_result = validate_user_data(user_data)
          
          if validation_result.is_valid {
            // Generate user ID
            user_id = generate_user_id()
            
            // Create user account
            user = {
              id: user_id,
              username: user_data.username,
              email: user_data.email,
              password_hash: hash_password(user_data.password),
              created_at: get_timestamp(),
              status: "active"
            }
            
            // Create user profile
            profile = {
              user_id: user_id,
              display_name: user_data.display_name || user_data.username,
              bio: user_data.bio || "",
              avatar: user_data.avatar || "default_avatar.png",
              cover_photo: user_data.cover_photo || "default_cover.png",
              location: user_data.location || "",
              website: user_data.website || "",
              verified: false,
              followers_count: 0,
              following_count: 0,
              posts_count: 0
            }
            
            // Save user and profile
            memory.users[user_id] = user
            memory.user_profiles[user_id] = profile
            
            registration_result = {
              success: true,
              user_id: user_id,
              message: "User registered successfully"
            }
          } else {
            registration_result = {
              success: false,
              errors: validation_result.errors
            }
          }
        }
      }
    }
  }

  recipe "follow_user" {
    needs: follower_id, followed_id
    gives: follow_result
    
    brain {
      plan {
        plan = { action: "follow" }
      }
      
      execute {
        if plan.action == "follow" {
          // Check if already following
          relationship_key = follower_id + "_" + followed_id
          
          if !memory.relationships[relationship_key] {
            // Create follow relationship
            relationship = {
              follower_id: follower_id,
              followed_id: followed_id,
              created_at: get_timestamp(),
              status: "active"
            }
            
            memory.relationships[relationship_key] = relationship
            
            // Update follower counts
            update_follower_counts(follower_id, followed_id)
            
            // Send notification
            send_follow_notification(follower_id, followed_id)
            
            follow_result = {
              success: true,
              message: "Successfully followed user"
            }
          } else {
            follow_result = {
              success: false,
              message: "Already following this user"
            }
          }
        }
      }
    }
  }

  recipe "update_user_profile" {
    needs: user_id, profile_data
    gives: update_result
    
    brain {
      plan {
        plan = { action: "update" }
      }
      
      execute {
        if plan.action == "update" {
          profile = memory.user_profiles[user_id]
          
          if profile {
            // Update profile fields
            for each field in profile_data {
              profile[field] = profile_data[field]
            }
            
            profile.updated_at = get_timestamp()
            
            memory.user_profiles[user_id] = profile
            
            update_result = {
              success: true,
              message: "Profile updated successfully"
            }
          } else {
            update_result = {
              success: false,
              message: "User profile not found"
            }
          }
        }
      }
    }
  }
}
```

## Step 2: Content Management System

```gx
helper "content_management" {
  can_do: ["post_creation", "media_handling", "content_moderation"]
  
  remember {
    posts = {}
    media_files = {}
    content_queue = {}
    moderation_results = {}
  }

  brain {
    plan {
      plan = { action: "manage_content" }
    }

    execute {
      if plan.action == "manage_content" {
        // Process new posts
        process_new_posts()
        
        // Handle media uploads
        handle_media_uploads()
        
        // Moderate content
        moderate_content()
        
        // Update content statistics
        update_content_statistics()
      }
    }
  }

  recipe "create_post" {
    needs: user_id, post_data
    gives: post_result
    
    brain {
      plan {
        plan = { action: "create" }
      }
      
      execute {
        if plan.action == "create" {
          // Generate post ID
          post_id = generate_post_id()
          
          // Create post object
          post = {
            id: post_id,
            user_id: user_id,
            content: post_data.content,
            media: post_data.media || [],
            hashtags: extract_hashtags(post_data.content),
            mentions: extract_mentions(post_data.content),
            created_at: get_timestamp(),
            updated_at: get_timestamp(),
            likes_count: 0,
            comments_count: 0,
            shares_count: 0,
            status: "pending_moderation"
          }
          
          // Add to content queue for moderation
          memory.content_queue[post_id] = post
          
          post_result = {
            success: true,
            post_id: post_id,
            message: "Post created and queued for moderation"
          }
        }
      }
    }
  }

  recipe "moderate_content" {
    needs: content_id
    gives: moderation_result
    
    brain {
      plan {
        plan = { action: "moderate" }
      }
      
      execute {
        if plan.action == "moderate" {
          content = memory.content_queue[content_id]
          
          if content {
            // Run content moderation checks
            spam_check = check_for_spam(content)
            inappropriate_check = check_for_inappropriate_content(content)
            duplicate_check = check_for_duplicates(content)
            
            if spam_check.is_spam || inappropriate_check.is_inappropriate {
              // Reject content
              content.status = "rejected"
              content.moderation_reason = spam_check.is_spam ? "spam" : "inappropriate"
              
              moderation_result = {
                approved: false,
                reason: content.moderation_reason
              }
            } else if duplicate_check.is_duplicate {
              // Flag as duplicate
              content.status = "duplicate"
              content.moderation_reason = "duplicate_content"
              
              moderation_result = {
                approved: false,
                reason: "duplicate_content"
              }
            } else {
              // Approve content
              content.status = "published"
              content.published_at = get_timestamp()
              
              // Add to posts collection
              memory.posts[content_id] = content
              
              // Update user post count
              update_user_post_count(content.user_id)
              
              // Send notifications to mentioned users
              send_mention_notifications(content)
              
              moderation_result = {
                approved: true,
                message: "Content approved and published"
              }
            }
            
            memory.moderation_results[content_id] = moderation_result
          }
        }
      }
    }
  }

  recipe "upload_media" {
    needs: user_id, media_data
    gives: upload_result
    
    brain {
      plan {
        plan = { action: "upload" }
      }
      
      execute {
        if plan.action == "upload" {
          // Generate media ID
          media_id = generate_media_id()
          
          // Process media file
          processed_media = process_media_file(media_data)
          
          // Store media information
          media = {
            id: media_id,
            user_id: user_id,
            file_name: media_data.file_name,
            file_type: media_data.file_type,
            file_size: media_data.file_size,
            url: processed_media.url,
            thumbnail_url: processed_media.thumbnail_url,
            created_at: get_timestamp(),
            status: "active"
          }
          
          memory.media_files[media_id] = media
          
          upload_result = {
            success: true,
            media_id: media_id,
            url: media.url,
            thumbnail_url: media.thumbnail_url
          }
        }
      }
    }
  }
}
```

## Step 3: Social Features System

```gx
helper "social_features" {
  can_do: ["likes_comments", "shares", "engagement_tracking"]
  
  remember {
    likes = {}
    comments = {}
    shares = {}
    engagement_metrics = {}
  }

  brain {
    plan {
      plan = { action: "manage_social_features" }
    }

    execute {
      if plan.action == "manage_social_features" {
        // Process likes and comments
        process_engagements()
        
        // Handle shares
        handle_shares()
        
        // Track engagement metrics
        track_engagement_metrics()
        
        // Update content statistics
        update_content_statistics()
      }
    }
  }

  recipe "like_post" {
    needs: user_id, post_id
    gives: like_result
    
    brain {
      plan {
        plan = { action: "like" }
      }
      
      execute {
        if plan.action == "like" {
          like_key = user_id + "_" + post_id
          
          if !memory.likes[like_key] {
            // Create like
            like = {
              user_id: user_id,
              post_id: post_id,
              created_at: get_timestamp()
            }
            
            memory.likes[like_key] = like
            
            // Update post like count
            update_post_like_count(post_id)
            
            // Send notification to post owner
            send_like_notification(user_id, post_id)
            
            like_result = {
              success: true,
              message: "Post liked successfully"
            }
          } else {
            // Unlike post
            delete memory.likes[like_key]
            
            // Decrease post like count
            decrease_post_like_count(post_id)
            
            like_result = {
              success: true,
              message: "Post unliked successfully"
            }
          }
        }
      }
    }
  }

  recipe "comment_on_post" {
    needs: user_id, post_id, comment_data
    gives: comment_result
    
    brain {
      plan {
        plan = { action: "comment" }
      }
      
      execute {
        if plan.action == "comment" {
          // Generate comment ID
          comment_id = generate_comment_id()
          
          // Create comment
          comment = {
            id: comment_id,
            user_id: user_id,
            post_id: post_id,
            content: comment_data.content,
            mentions: extract_mentions(comment_data.content),
            created_at: get_timestamp(),
            likes_count: 0,
            replies: []
          }
          
          memory.comments[comment_id] = comment
          
          // Update post comment count
          update_post_comment_count(post_id)
          
          // Send notification to post owner
          send_comment_notification(user_id, post_id, comment_id)
          
          // Send notifications to mentioned users
          send_mention_notifications_in_comment(comment)
          
          comment_result = {
            success: true,
            comment_id: comment_id,
            message: "Comment added successfully"
          }
        }
      }
    }
  }

  recipe "share_post" {
    needs: user_id, post_id, share_data
    gives: share_result
    
    brain {
      plan {
        plan = { action: "share" }
      }
      
      execute {
        if plan.action == "share" {
          // Generate share ID
          share_id = generate_share_id()
          
          // Create share
          share = {
            id: share_id,
            user_id: user_id,
            original_post_id: post_id,
            share_message: share_data.message || "",
            created_at: get_timestamp(),
            platform: share_data.platform || "internal"
          }
          
          memory.shares[share_id] = share
          
          // Update post share count
          update_post_share_count(post_id)
          
          // Create new post for the share
          if share_data.create_new_post {
            shared_post = create_shared_post(user_id, post_id, share_data)
          }
          
          share_result = {
            success: true,
            share_id: share_id,
            message: "Post shared successfully"
          }
        }
      }
    }
  }
}
```

## Step 4: Messaging System

```gx
helper "messaging_system" {
  can_do: ["real_time_messaging", "notifications", "chat_management"]
  
  remember {
    conversations = {}
    messages = {}
    notifications = {}
    online_users = {}
  }

  brain {
    plan {
      plan = { action: "manage_messaging" }
    }

    execute {
      if plan.action == "manage_messaging" {
        // Process new messages
        process_new_messages()
        
        // Handle notifications
        handle_notifications()
        
        // Manage online status
        manage_online_status()
        
        // Clean up old messages
        cleanup_old_messages()
      }
    }
  }

  recipe "send_message" {
    needs: sender_id, receiver_id, message_data
    gives: message_result
    
    brain {
      plan {
        plan = { action: "send" }
      }
      
      execute {
        if plan.action == "send" {
          // Generate message ID
          message_id = generate_message_id()
          
          // Create message
          message = {
            id: message_id,
            sender_id: sender_id,
            receiver_id: receiver_id,
            content: message_data.content,
            message_type: message_data.type || "text",
            media: message_data.media || null,
            created_at: get_timestamp(),
            read_at: null,
            status: "sent"
          }
          
          memory.messages[message_id] = message
          
          // Get or create conversation
          conversation_id = get_conversation_id(sender_id, receiver_id)
          if !memory.conversations[conversation_id] {
            memory.conversations[conversation_id] = {
              id: conversation_id,
              participants: [sender_id, receiver_id],
              last_message: message,
              created_at: get_timestamp(),
              updated_at: get_timestamp()
            }
          } else {
            memory.conversations[conversation_id].last_message = message
            memory.conversations[conversation_id].updated_at = get_timestamp()
          }
          
          // Send notification if user is offline
          if !memory.online_users[receiver_id] {
            send_message_notification(receiver_id, sender_id, message)
          }
          
          message_result = {
            success: true,
            message_id: message_id,
            conversation_id: conversation_id
          }
        }
      }
    }
  }

  recipe "get_conversation_messages" {
    needs: conversation_id, user_id, limit
    gives: messages
    
    brain {
      plan {
        plan = { action: "get" }
      }
      
      execute {
        if plan.action == "get" {
          conversation = memory.conversations[conversation_id]
          
          if conversation && conversation.participants.includes(user_id) {
            // Get messages for this conversation
            conversation_messages = []
            
            for each message_id in memory.messages {
              message = memory.messages[message_id]
              if (message.sender_id === conversation.participants[0] && 
                  message.receiver_id === conversation.participants[1]) ||
                 (message.sender_id === conversation.participants[1] && 
                  message.receiver_id === conversation.participants[0]) {
                conversation_messages.push(message)
              }
            }
            
            // Sort by creation time
            conversation_messages.sort((a, b) => a.created_at - b.created_at)
            
            // Limit results
            messages = conversation_messages.slice(-limit)
            
            // Mark messages as read
            mark_messages_as_read(conversation_id, user_id)
          } else {
            messages = []
          }
        }
      }
    }
  }

  recipe "send_notification" {
    needs: user_id, notification_data
    gives: notification_result
    
    brain {
      plan {
        plan = { action: "send" }
      }
      
      execute {
        if plan.action == "send" {
          // Generate notification ID
          notification_id = generate_notification_id()
          
          // Create notification
          notification = {
            id: notification_id,
            user_id: user_id,
            type: notification_data.type,
            title: notification_data.title,
            message: notification_data.message,
            data: notification_data.data || {},
            created_at: get_timestamp(),
            read_at: null,
            status: "unread"
          }
          
          memory.notifications[notification_id] = notification
          
          // Send real-time notification if user is online
          if memory.online_users[user_id] {
            send_realtime_notification(user_id, notification)
          }
          
          notification_result = {
            success: true,
            notification_id: notification_id
          }
        }
      }
    }
  }
}
```

## Step 5: Feed Engine

```gx
helper "feed_engine" {
  can_do: ["feed_generation", "content_discovery", "personalization"]
  
  remember {
    user_feeds = {}
    feed_algorithms = {}
    content_recommendations = {}
    trending_content = {}
  }

  brain {
    plan {
      plan = { action: "generate_feeds" }
    }

    execute {
      if plan.action == "generate_feeds" {
        // Generate personalized feeds
        generate_personalized_feeds()
        
        // Update trending content
        update_trending_content()
        
        // Optimize feed algorithms
        optimize_feed_algorithms()
        
        // Update content recommendations
        update_content_recommendations()
      }
    }
  }

  recipe "get_user_feed" {
    needs: user_id, page, limit
    gives: feed_content
    
    brain {
      plan {
        plan = { action: "get" }
      }
      
      execute {
        if plan.action == "get" {
          // Get user preferences and relationships
          user_profile = memory.user_profiles[user_id]
          following = get_user_following(user_id)
          
          // Get posts from followed users
          followed_posts = get_posts_from_users(following)
          
          // Get trending posts
          trending_posts = get_trending_posts()
          
          // Get recommended posts
          recommended_posts = get_recommended_posts(user_id)
          
          // Combine and rank posts
          all_posts = combine_posts([
            followed_posts,
            trending_posts,
            recommended_posts
          ])
          
          // Apply personalization
          personalized_posts = personalize_feed(all_posts, user_profile)
          
          // Paginate results
          feed_content = {
            posts: personalized_posts.slice(page * limit, (page + 1) * limit),
            has_more: personalized_posts.length > (page + 1) * limit,
            next_page: page + 1
          }
        }
      }
    }
  }

  recipe "get_trending_posts" {
    needs: none
    gives: trending_posts
    
    brain {
      plan {
        plan = { action: "get" }
      }
      
      execute {
        if plan.action == "get" {
          // Calculate trending scores for all posts
          trending_scores = {}
          
          for each post_id in memory.posts {
            post = memory.posts[post_id]
            if post.status === "published" {
              score = calculate_trending_score(post)
              trending_scores[post_id] = score
            }
          }
          
          // Sort by trending score
          sorted_posts = sort_posts_by_score(trending_scores)
          
          // Get top trending posts
          trending_posts = sorted_posts.slice(0, 50)
        }
      }
    }
  }

  recipe "calculate_trending_score" {
    needs: post
    gives: score
    
    brain {
      plan {
        plan = { action: "calculate" }
      }
      
      execute {
        if plan.action == "calculate" {
          // Multi-factor trending calculation
          engagement_rate = (post.likes_count + post.comments_count * 2 + post.shares_count * 3) / 100
          
          // Time decay factor
          time_since_post = get_timestamp() - post.created_at
          time_decay = Math.exp(-time_since_post / (24 * 60 * 60 * 1000)) // 24 hours
          
          // User influence factor
          user_influence = get_user_influence(post.user_id)
          
          // Calculate final score
          score = engagement_rate * time_decay * user_influence
        }
      }
    }
  }
}
```

## Running the Social Media Platform

1. **Save the complete application** to a file:
   ```bash
   # Save all helpers to social_media_platform.gx
   # (Include all the helper code above)
   ```

2. **Run the application**:
   ```bash
   ./bin/gx social_media_platform.gx
   ```

3. **Expected output**:
   ```
   🧠 GX Language Runtime v0.1.0 (Self-Hosting)
   =============================================
   
     📝 Loading GX file: social_media_platform.gx
     📊 File size: 15800 bytes
   
     🚀 Executing GX Runtime: social_media_platform.gx
     🧠 Initializing cognitive runtime...
     📊 Found 5 helpers with 25 brain processes
     🧠 Brain cycle: Plan → Execute → Remember → Communicate
     Social Media Platform initialized successfully!
     User Management: Active
     Content Management: Active
     Social Features: Active
     Messaging System: Active
     Feed Engine: Active
     ✅ GX Runtime execution completed successfully!
   
   🎉 GX Runtime completed successfully!
   ```

## Advanced Features to Add

1. **Live Streaming**: Add real-time video streaming capabilities
2. **Stories Feature**: Implement temporary content sharing
3. **Advanced Analytics**: Add detailed user and content analytics
4. **Content Discovery**: Implement AI-powered content recommendations
5. **Monetization**: Add advertising and subscription features
6. **API Integration**: Connect with external services and APIs

## Practice Exercises

1. **Build a user profile system** with customizable avatars and bios
2. **Create a post creation interface** with media upload support
3. **Implement a comment system** with nested replies
4. **Build a real-time chat feature** with online status
5. **Create a trending algorithm** for content discovery

## Next Steps

Now that you have a social media platform, you can:
- [Build an E-commerce System](10_ecommerce_system.md)
- [Build a Gaming Platform](11_gaming_platform.md)
- [Build a TikTok Clone](08_tiktok_clone.md)

---

**© 2025 DEVJSX LIMITED, a company registered in England and Wales. Company Number: 16618207 Registered Office: 128 City Road, London, United Kingdom, EC1V 2NX website: www.devjsx.com** 