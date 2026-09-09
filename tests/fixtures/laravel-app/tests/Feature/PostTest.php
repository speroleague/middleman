<?php

namespace Tests\Feature;

use App\Models\Post;
use Tests\TestCase;

class PostTest extends TestCase
{
    public function test_index_only_returns_published_posts(): void
    {
        Post::create(['title' => 'Draft', 'body' => 'wip', 'published_at' => null]);
        Post::create(['title' => 'Live', 'body' => 'done', 'published_at' => now()]);

        $this->getJson('/api/posts')
            ->assertOk()
            ->assertJsonCount(1, 'data');
    }
}
