<?php

namespace App\Http\Controllers;

use App\Models\Post;
use Illuminate\Http\JsonResponse;

class PostController extends Controller
{
    public function index(): JsonResponse
    {
        return response()->json(Post::whereNotNull('published_at')->latest('published_at')->take(50)->get());
    }

    public function show(int $id): JsonResponse
    {
        $post = Post::findOrFail($id);
        return response()->json($post);
    }
}
